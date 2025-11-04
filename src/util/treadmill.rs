use std::collections::HashSet;
use std::mem::swap;
use std::sync::Mutex;

use crate::util::ObjectReference;

use super::object_enum::ObjectEnumerator;

/// A data structure for recording objects in the LOS.
///
/// It is divided into the nursery and the mature space, and each of them is further divided into
/// the from-space and the to-space.
///
/// All operations are protected by a single mutex [`TreadMill::sync`].
pub struct TreadMill {
    sync: Mutex<TreadMillSync>,
}

/// The synchronized part of [`TreadMill`]
#[derive(Default)]
struct TreadMillSync {
    nursery: SpacePair,
    mature: SpacePair,
}

/// A pair of from and two spaces.
#[derive(Default)]
struct SpacePair {
    from_space: HashSet<ObjectReference>,
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
            .field("nursery.from", &sync.nursery.from_space)
            .field("nursery.to", &sync.nursery.to_space)
            .field("mature.from", &sync.mature.from_space)
            .field("mature.to", &sync.mature.to_space)
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
    /// New objects are normally added to `nursery.to_space`.  But when allocatin as live (e.g. when
    /// concurrent marking is active), we directly add into `mature.to_space`.
    pub fn add_to_treadmill(&self, object: ObjectReference, nursery: bool) {
        let mut sync = self.sync.lock().unwrap();
        if nursery {
            trace!("Adding {} to nursery.to_space", object);
            sync.nursery.to_space.insert(object);
        } else {
            trace!("Adding {} to mature.to_space", object);
            sync.mature.to_space.insert(object);
        }
    }

    /// Take all objects from the `nursery.from_space`.  This is called during sweeping at which time
    /// all objects in the from-space are unreachable.
    pub fn collect_nursery(&self) -> impl IntoIterator<Item = ObjectReference> {
        let mut sync = self.sync.lock().unwrap();
        std::mem::take(&mut sync.nursery.from_space)
    }

    /// Take all objects from the `mature.from_space`.  This is called during sweeping at which time
    /// all objects in the from-space are unreachable.
    pub fn collect_mature(&self) -> impl IntoIterator<Item = ObjectReference> {
        let mut sync = self.sync.lock().unwrap();
        std::mem::take(&mut sync.mature.from_space)
    }

    /// Move an object to `mature.to_space`.  Called when an object is determined to be reachable.
    pub fn copy(&self, object: ObjectReference, is_in_nursery: bool) {
        let mut sync = self.sync.lock().unwrap();
        if is_in_nursery {
            debug_assert!(
                sync.nursery.from_space.contains(&object),
                "copy source object ({}) must be in nursery.from_space",
                object
            );
            sync.nursery.from_space.remove(&object);
        } else {
            debug_assert!(
                sync.mature.from_space.contains(&object),
                "copy source object ({}) must be in mature.from_space",
                object
            );
            sync.mature.from_space.remove(&object);
        }
        sync.mature.to_space.insert(object);
    }

    /// Return true if the nursery from-space is empty.
    pub fn is_nursery_from_space_empty(&self) -> bool {
        let sync = self.sync.lock().unwrap();
        sync.nursery.from_space.is_empty()
    }

    /// Return true if the nursery to-space is empty.
    pub fn is_nursery_to_space_empty(&self) -> bool {
        let sync = self.sync.lock().unwrap();
        sync.nursery.to_space.is_empty()
    }

    /// Return true if the mature from-space is empty.
    pub fn is_mature_from_space_empty(&self) -> bool {
        let sync = self.sync.lock().unwrap();
        sync.mature.from_space.is_empty()
    }

    /// Flip the from- and to-spaces.
    ///
    /// `full_heap` is true during full-heap GC, or false during nursery GC.
    pub fn flip(&mut self, full_heap: bool) {
        let sync = self.sync.get_mut().unwrap();
        swap(&mut sync.nursery.from_space, &mut sync.nursery.to_space);
        trace!("Flipped nursery.from_space and nursery.to_space");
        if full_heap {
            swap(&mut sync.mature.from_space, &mut sync.mature.to_space);
            trace!("Flipped mature.from_space and mature.to_space");
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
        visit_objects(&sync.nursery.to_space);
        visit_objects(&sync.mature.to_space);

        match from_space_policy {
            FromSpacePolicy::Include => {
                visit_objects(&sync.nursery.from_space);
                visit_objects(&sync.mature.from_space);
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
                assert!(
                    sync.nursery.from_space.is_empty(),
                    "nursery.from_space is not empty"
                );
                assert!(
                    sync.mature.from_space.is_empty(),
                    "mature.from_space is not empty"
                );
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
