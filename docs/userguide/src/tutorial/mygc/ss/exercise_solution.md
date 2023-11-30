# Triplespace backup instructions

This is *one* possible implementation of the Triplespace collector, provided
in case you are stuck on the exercise.

**Attempt the exercise yourself before reading this.**

First, rename all instances of `mygc` to `triplespace`, and add it as a
module by following the instructions in [Create MyGC](../create.md).

In `triplespace/global.rs`:

 1. Add a `youngspace` field to `pub struct TripleSpace`:

       ```rust
       pub struct TripleSpace<VM: VMBinding> {
          pub hi: AtomicBool,
          pub copyspace0: CopySpace<VM>,
          pub copyspace1: CopySpace<VM>,
          pub youngspace: CopySpace<VM>, // Add this!
          pub common: CommonPlan<VM>,
      }
      ```

 2. Define the parameters for the youngspace in `new()` in
 `Plan for TripleSpace`:
      ```rust
      fn new(
         vm_map: &'static VMMap,
         mmapper: &'static Mmapper,
         options: Arc<UnsafeOptionsWrapper>,
         _scheduler: &'static MMTkScheduler<Self::VM>,
     ) -> Self {
         //change - again, completely changed.
         let mut heap = HeapMeta::new(HEAP_START, HEAP_END);

         TripleSpace {
             hi: AtomicBool::new(false),
             copyspace0: CopySpace::new(
                 "copyspace0",
                 false,
                 true,
                 VMRequest::discontiguous(),
                 vm_map,
                 mmapper,
                 &mut heap,
             ),
             copyspace1: CopySpace::new(
                 "copyspace1",
                 true,
                 true,
                 VMRequest::discontiguous(),
                 vm_map,
                 mmapper,
                 &mut heap,
             ),

             // Add this!
             youngspace: CopySpace::new(
                 "youngspace",
                 true,
                 true,
                 VMRequest::discontiguous(),
                 vm_map,
                 mmapper,
                 &mut heap,
             ),
             common: CommonPlan::new(vm_map, mmapper, options, heap, &TRIPLESPACE_CONSTRAINTS, &[]),
         }
     }
      ```
 3. Initialise the youngspace in `gc_init()`:
     ```rust
      fn gc_init(
         &mut self,
         heap_size: usize,
         vm_map: &'static VMMap,
         scheduler: &Arc<MMTkScheduler<VM>>,
     ) {
         self.common.gc_init(heap_size, vm_map, scheduler);
         self.copyspace0.init(&vm_map);
         self.copyspace1.init(&vm_map);
         self.youngspace.init(&vm_map); // Add this!
     }
     ```
 4. Prepare the youngspace (as a fromspace) in `prepare()`:
     ```rust
     fn prepare(&self, tls: OpaquePointer) {
        self.common.prepare(tls, true);
        self.hi
            .store(!self.hi.load(Ordering::SeqCst), Ordering::SeqCst);
        let hi = self.hi.load(Ordering::SeqCst);
        self.copyspace0.prepare(hi);
        self.copyspace1.prepare(!hi);
        self.youngspace.prepare(true); // Add this!
    }
     ```
 5. Release the youngspace in `release()`:
     ```rust
     fn release(&self, tls: OpaquePointer) {
        self.common.release(tls, true);
        self.fromspace().release();
        self.youngspace().release(); // Add this!
    }
     ```
 6. Under the reference functions `tospace()` and `fromspace()`, add a similar
 reference function `youngspace()`:
     ```rust
     pub fn youngspace(&self) -> &CopySpace<VM> {
        &self.youngspace
    }
     ```

In `mutator.rs`:
 1. Map a bump pointer to the youngspace (replacing the one mapped to the
  tospace) in `space_mapping` in `create_triplespace_mutator()`:
     ```rust
     space_mapping: box vec![
         (AllocatorSelector::BumpPointer(0), plan.youngspace()), // Change this!
         (
             AllocatorSelector::BumpPointer(1),
             plan.common.get_immortal(),
         ),
         (AllocatorSelector::LargeObject(0), plan.common.get_los()),
     ],
     ```
 2. Rebind the bump pointer to youngspace (rather than the tospace) in
 `triplespace_mutator_release()`:
     ```rust
     pub fn triplespace_mutator_release<VM: VMBinding> (
         mutator: &mut Mutator<VM>,
         _tls: OpaquePointer
     ) {
         let bump_allocator = unsafe {
             mutator
                 .allocators
                 . get_allocator_mut(
                     mutator.config.allocator_mapping[AllocationType::Default]
                 )
             }
             .downcast_mut::<BumpAllocator<VM>>()
             .unwrap();
             bump_allocator.rebind(Some(mutator.plan.youngspace())); // Change this!
     }
     ```

In `gc_work.rs`:
1. Add the youngspace to trace_object, following the same format as
 the tospace and fromspace:
    ```rust
        fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
            debug_assert!(!object.is_null());

            // Add this!
            else if self.plan().youngspace().in_space(object) {
                self.plan().youngspace.trace_object::<Self, TripleSpaceCopyContext<VM>>(
                    self,
                    object,
                    super::global::ALLOC_TripleSpace,
                    unsafe { self.worker().local::<TripleSpaceCopyContext<VM>>() },
                )
            }

            else if self.plan().tospace().in_space(object) {
                self.plan().tospace().trace_object::<Self, TripleSpaceCopyContext<VM>>(
                    self,
                    object,
                    super::global::ALLOC_TripleSpace,
                    unsafe { self.worker().local::<MyGCCopyContext<VM>>() },
                )
            } else if self.plan().fromspace().in_space(object) {
                self.plan().fromspace().trace_object::<Self, MyGCCopyContext<VM>>(
                    self,
                    object,
                    super::global::ALLOC_TripleSpace,
                    unsafe { self.worker().local::<TripleSpaceCopyContext<VM>>() },
                )
            } else {
                self.plan().common.trace_object::<Self, TripleSpaceCopyContext<VM>>(self, object)
            }
        }
    }
    ```
