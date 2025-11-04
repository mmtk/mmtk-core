use std::collections::HashSet;
use std::mem::swap;
use std::sync::Mutex;

use crate::util::ObjectReference;

use super::object_enum::ObjectEnumerator;

/// A data structure for recording objects in the LOS.
///
/// All operations are protected by a single mutex [`TreadMill::sync`].
pub struct TreadMill {
    sync: Mutex<TreadMillSync>,
}

/// The synchronized part of [`TreadMill`]
#[derive(Default)]
struct TreadMillSync {
    /// The from-space.  During GC, it contains old objects with unknown liveness.
    from_space: HashSet<ObjectReference>,
    /// The to-space.  During mutator time, it contains old objects; during GC, it contains objects
    /// determined to be live.
    to_space: HashSet<ObjectReference>,
    /// The collection nursery.  During GC, it contains young objects with unknown liveness.
    collect_nursery: HashSet<ObjectReference>,
    /// The allocation nursery.  During mutator time, it contains young objects; during GC, it
    /// remains empty.
    alloc_nursery: HashSet<ObjectReference>,
}

impl std::fmt::Debug for TreadMill {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sync = self.sync.lock().unwrap();
        f.debug_struct("TreadMill")
            .field("from_space", &sync.from_space)
            .field("to_space", &sync.to_space)
            .field("collect_nursery", &sync.collect_nursery)
            .field("alloc_nursery", &sync.alloc_nursery)
            .finish()
    }
}

impl TreadMill {
    pub fn new() -> Self {
        TreadMill {
            sync: Mutex::new(Default::default()),
        }
    }

    /// Add an object to the treadmill.
    ///
    /// New objects are normally added to `alloc_nursery`.  But when allocating as live (e.g. when
    /// concurrent marking is active), we directly add into the `to_space`.
    pub fn add_to_treadmill(&self, object: ObjectReference, nursery: bool) {
        let mut sync = self.sync.lock().unwrap();
        if nursery {
            trace!("Adding {} to alloc_nursery", object);
            sync.alloc_nursery.insert(object);
        } else {
            trace!("Adding {} to to_space", object);
            sync.to_space.insert(object);
        }
    }

    /// Take all objects from the `collect_nursery`.  This is called during sweeping at which time
    /// all unreachable young objects are in the collection nursery.
    pub fn collect_nursery(&self) -> impl IntoIterator<Item = ObjectReference> {
        let mut sync = self.sync.lock().unwrap();
        std::mem::take(&mut sync.collect_nursery)
    }

    /// Take all objects from the `from_space`.  This is called during sweeping at which time all
    /// unreachable old objects are in the from-space.
    pub fn collect_mature(&self) -> impl IntoIterator<Item = ObjectReference> {
        let mut sync = self.sync.lock().unwrap();
        std::mem::take(&mut sync.from_space)
    }

    /// Move an object to `to_space`.  Called when an object is determined to be reachable.
    pub fn copy(&self, object: ObjectReference, is_in_nursery: bool) {
        let mut sync = self.sync.lock().unwrap();
        if is_in_nursery {
            debug_assert!(
                sync.collect_nursery.contains(&object),
                "copy source object ({}) must be in collect_nursery",
                object
            );
            sync.collect_nursery.remove(&object);
        } else {
            debug_assert!(
                sync.from_space.contains(&object),
                "copy source object ({}) must be in from_space",
                object
            );
            sync.from_space.remove(&object);
        }
        sync.to_space.insert(object);
    }

    /// Return true if the to-space is empty.
    pub fn is_to_space_empty(&self) -> bool {
        let sync = self.sync.lock().unwrap();
        sync.to_space.is_empty()
    }

    /// Return true if the from-space is empty.
    pub fn is_from_space_empty(&self) -> bool {
        let sync = self.sync.lock().unwrap();
        sync.from_space.is_empty()
    }

    /// Return true if the allocation nursery is empty.
    pub fn is_alloc_nursery_empty(&self) -> bool {
        let sync = self.sync.lock().unwrap();
        sync.alloc_nursery.is_empty()
    }

    /// Return true if the collection nursery is empty.
    pub fn is_collect_nursery_empty(&self) -> bool {
        let sync = self.sync.lock().unwrap();
        sync.collect_nursery.is_empty()
    }

    /// Flip object sets.
    ///
    /// It will flip the allocation nursery and the collection nursery.
    ///
    /// If `full_heap` is true, it will also flip the from-space and the to-space.
    pub fn flip(&mut self, full_heap: bool) {
        let sync = self.sync.get_mut().unwrap();
        swap(&mut sync.alloc_nursery, &mut sync.collect_nursery);
        trace!("Flipped alloc_nursery and collect_nursery");
        if full_heap {
            swap(&mut sync.from_space, &mut sync.to_space);
            trace!("Flipped from_space and to_space");
        }
    }

    /// Enumerate objects.
    ///
    /// Objects in the allocation nursery and the to-spaces are always enumerated.  They include all
    /// objects during mutator time, and objects determined to be live during a GC.
    ///
    /// If `all` is true, it will enumerate the collection nursery and the from-space, too.
    pub(crate) fn enumerate_objects(&self, enumerator: &mut dyn ObjectEnumerator, all: bool) {
        let sync = self.sync.lock().unwrap();
        let mut enumerated = 0usize;
        let mut visit_objects = |set: &HashSet<ObjectReference>| {
            for object in set.iter() {
                enumerator.visit_object(*object);
                enumerated += 1;
            }
        };
        visit_objects(&sync.alloc_nursery);
        visit_objects(&sync.to_space);

        if all {
            visit_objects(&sync.collect_nursery);
            visit_objects(&sync.from_space);
        }

        debug!("Enumerated {enumerated} objects in LOS.  all: {all}");
    }
}

impl Default for TreadMill {
    fn default() -> Self {
        Self::new()
    }
}
