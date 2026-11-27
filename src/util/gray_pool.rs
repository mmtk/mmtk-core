#![allow(dead_code)]
//! A plan-owned pool of grey (marked but not yet scanned) objects.
//!
//! Work packets push the objects they could not finish (because a deadline passed or a pause
//! interrupted them) into the pool, and later pauses or concurrent phases pop them again.
//! Unlike packets left in a work bucket, the pool can be inspected and rewritten, for
//! example to update references after objects have moved.

use crate::util::ObjectReference;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

#[derive(Default)]
pub(crate) struct GrayPool {
    chunks: Mutex<Vec<Vec<ObjectReference>>>,
    /// Number of objects in all chunks.
    objects: AtomicUsize,
}

impl GrayPool {
    /// Push a chunk. Empty chunks are dropped.
    pub fn push(&self, chunk: Vec<ObjectReference>) {
        if chunk.is_empty() {
            return;
        }
        self.objects.fetch_add(chunk.len(), Ordering::SeqCst);
        self.chunks.lock().unwrap().push(chunk);
    }

    /// Pop one chunk, if any.
    pub fn pop(&self) -> Option<Vec<ObjectReference>> {
        let chunk = self.chunks.lock().unwrap().pop()?;
        self.objects.fetch_sub(chunk.len(), Ordering::SeqCst);
        Some(chunk)
    }

    /// Remove and return all chunks.
    pub fn take_all(&self) -> Vec<Vec<ObjectReference>> {
        let all = std::mem::take(&mut *self.chunks.lock().unwrap());
        let n: usize = all.iter().map(|c| c.len()).sum();
        self.objects.fetch_sub(n, Ordering::SeqCst);
        all
    }

    /// Return `true` if no object is in the pool.
    pub fn is_empty(&self) -> bool {
        self.objects.load(Ordering::SeqCst) == 0
    }

    /// Number of objects in the pool.
    pub fn len(&self) -> usize {
        self.objects.load(Ordering::SeqCst)
    }

    /// Rewrite every entry: `f` returns the new reference, or `None` to drop the entry.
    /// Only call this while no other thread uses the pool.
    pub fn filter_map_in_place(
        &self,
        mut f: impl FnMut(ObjectReference) -> Option<ObjectReference>,
    ) {
        let mut chunks = self.chunks.lock().unwrap();
        let mut total = 0;
        for chunk in chunks.iter_mut() {
            let rewritten: Vec<ObjectReference> = chunk.drain(..).filter_map(&mut f).collect();
            *chunk = rewritten;
            total += chunk.len();
        }
        chunks.retain(|c| !c.is_empty());
        self.objects.store(total, Ordering::SeqCst);
    }
}
