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
    /// The nursery.  During mutator time, newly allocated objects are added to the nursery.  After
    /// a GC, the nursery will be evacuated.
    nursery: HashSet<ObjectReference>,
    /// The from-space.  During GC, old objects whose liveness are not yet determined are kept in
    /// the from-space.  After GC, the from-space will be evacuated.
    from_space: HashSet<ObjectReference>,
    /// The to-space.  It holds old objects during mutator time.  Objects in the to-space are moved
    /// to the from-space at the beginning of GC, and objects are moved to the to-space once they
    /// are determined to be live.
    to_space: HashSet<ObjectReference>,
}

/// Used by [`TreadMill::enumerate_objects`] to determine what to do to objects in the from-spaces.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FromSpacePolicy {
    /// Also enumerate objects in the from-spaces.  Useful when setting object metadata during
    /// `Prepare`, at which time we may have swapped the from- and to-spaces.
    Include,
    /// Silently skip objects in the from-spaces.  Useful when enumerating live objects after the
    /// liveness of objects is determined and live objects have been moved to the to-spaces.  One
    /// use case is for forwarding references in some mark-compact GC algorithms.
    Skip,
    /// Assert that from-spaces must be empty.  Useful when the mutator calls
    /// `MMTK::enumerate_objects`, at which time GC must not be in progress and the from-spaces must
    /// be empty.
    ExpectEmpty,
}

impl std::fmt::Debug for TreadMill {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sync = self.sync.lock().unwrap();
        f.debug_struct("TreadMill")
            .field("nursery", &sync.nursery)
            .field("from", &sync.from_space)
            .field("to", &sync.to_space)
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
    /// New objects are normally added to `nursery.to_space`.  But when allocating as live (e.g.
    /// when concurrent marking is active), we directly add into the `to_space`.
    pub fn add_to_treadmill(&self, object: ObjectReference, allocate_as_live: bool) {
        let mut sync = self.sync.lock().unwrap();
        if allocate_as_live {
            trace!("Adding {} to to_space", object);
            sync.to_space.insert(object);
        } else {
            trace!("Adding {} to nursery", object);
            sync.nursery.insert(object);
        }
    }

    /// Take all objects from the `nursery`.  This is called during sweeping at which time all
    /// objects in the nursery are unreachable.
    pub fn collect_nursery(&self) -> impl IntoIterator<Item = ObjectReference> {
        let mut sync = self.sync.lock().unwrap();
        std::mem::take(&mut sync.nursery)
    }

    /// Take all objects from the `from_space`.  This is called during sweeping at which time all
    /// objects in the from-space are unreachable.
    pub fn collect_mature(&self) -> impl IntoIterator<Item = ObjectReference> {
        let mut sync = self.sync.lock().unwrap();
        std::mem::take(&mut sync.from_space)
    }

    /// Move an object to `to_space`.  Called when an object is determined to be reachable.
    pub fn copy(&self, object: ObjectReference, is_in_nursery: bool) {
        let mut sync = self.sync.lock().unwrap();
        if is_in_nursery {
            debug_assert!(
                sync.nursery.contains(&object),
                "copy source object ({}) must be in nursery",
                object
            );
            sync.nursery.remove(&object);
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

    /// Return true if the nursery is empty.
    pub fn is_nursery_empty(&self) -> bool {
        let sync = self.sync.lock().unwrap();
        sync.nursery.is_empty()
    }

    /// Return true if the from-space is empty.
    pub fn is_from_space_empty(&self) -> bool {
        let sync = self.sync.lock().unwrap();
        sync.from_space.is_empty()
    }

    /// Flip the from- and to-spaces.
    ///
    /// `full_heap` is true during full-heap GC, or false during nursery GC.
    pub fn flip(&mut self, full_heap: bool) {
        let sync = self.sync.get_mut().unwrap();
        if full_heap {
            swap(&mut sync.from_space, &mut sync.to_space);
            trace!("Flipped from_space and to_space");
        }
    }

    /// Enumerate objects.
    ///
    /// Objects in the to-spaces are always enumerated.  `from_space_policy` determines the action
    /// for objects in the nursery and mature from-spaces.
    pub(crate) fn enumerate_objects(
        &self,
        enumerator: &mut dyn ObjectEnumerator,
        from_space_policy: FromSpacePolicy,
    ) {
        let sync = self.sync.lock().unwrap();
        let mut enumerated = 0usize;
        let mut visit_objects = |set: &HashSet<ObjectReference>| {
            for object in set.iter() {
                enumerator.visit_object(*object);
                enumerated += 1;
            }
        };
        visit_objects(&sync.to_space);

        match from_space_policy {
            FromSpacePolicy::Include => {
                visit_objects(&sync.nursery);
                visit_objects(&sync.from_space);
            }
            FromSpacePolicy::Skip => {
                // Do nothing.
            }
            FromSpacePolicy::ExpectEmpty => {
                // Note that during concurrent GC (e.g. in ConcurrentImmix), object have been moved
                // to from-spaces, and GC workers are tracing objects concurrently, moving object to
                // `mature.to_space`.  If a mutator calls `MMTK::enumerate_objects` during
                // concurrent GC, the assertions below will fail.  That's expected because we
                // currently disallow the VM binding to call `MMTK::enumerate_objects` during any GC
                // activities, including concurrent GC.
                assert!(sync.nursery.is_empty(), "nursery is not empty");
                assert!(sync.from_space.is_empty(), "from_space is not empty");
            }
        }
        debug!("Enumerated {enumerated} objects in LOS.  from_space_policy={from_space_policy:?}");
    }
}

impl Default for TreadMill {
    fn default() -> Self {
        Self::new()
    }
}
