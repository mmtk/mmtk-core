# Collection: Implement garbage collection

## CopyContext and Scheduler

We need to add a few more things to get garbage collection working. 
Specifically, we need to add a `CopyContext`, which a GC worker uses for 
copying objects, and GC work packets that will be scheduled for a collection.

At the moment, none of the files in the plan are suited for garbage collection 
operations. So, we need to add a new file to hold the `CopyContext` and other 
structures and functions that will give the collector proper functionality.

1. Make a new file under `mygc`, called `gc_work.rs`.
2. In `mod.rs`, import `gc_work` as a module by adding the line `mod gc_work`.
3. In `gc_work.rs`, add the following import statements:
    ```rust
    use super::global::MyGC;
    use crate::policy::space::Space;
    use crate::scheduler::gc_work::*;
    use crate::vm::VMBinding;
    use crate::MMTK;
    use crate::plan::PlanConstraints;
    use crate::scheduler::WorkerLocal;
    ```

4. Add a new structure, `MyGCCopyContext`, with the type parameter 
`VM: VMBinding`. It should have the fields `plan: &'static MyGC<VM>`
and `mygc: BumpAllocator`.
   ```rust
   pub struct MyGCCopyContext<VM: VMBinding> {
       plan:&'static MyGC<VM>,
       mygc: BumpAllocator<VM>,
   }
   ```
   
5. Create an implementation block - 
`impl<VM: VMBinding> CopyContext for MyGCCopyContext<VM>`.
   1. Define the associate type `VM` for `CopyContext` as the VMBinding type 
   given to the class as `VM`: `type VM: VM`. 
   1. Add the following skeleton functions (taken from `plan/global.rs`):
       ```rust
       fn constraints(&self) -> &'static PlanConstraints {
           unimplemented!()
       }
       fn init(&mut self, tls: OpaquePointer) {
           unimplemented!()
       }
       fn prepare(&mut self) {
           unimplemented!()
       }
       fn release(&mut self) {
           unimplemented!()
       }
       fn alloc_copy(`init
           &mut self,
           original: ObjectReference,
           bytes: usize,
           align: usize,
           offset: isize,
           semantics: AllocationSemantics,
       ) -> Address {
           unimplemented!()
       }
       fn post_copy(
           &mut self,
           _obj: ObjectReference,
           _tib: Address,
           _bytes: usize,
           _semantics: AllocationSemantics,
       ) {
           unimplemented!()
       }
       ```
   1. In `init()`, set the `tls` variable in the held instance of `mygc` to
   the one passed to the function.
   1. In `constraints()`, return a reference of `MYGC_CONSTRAINTS`.
   1. We just leave the rest of the functions empty for now and will implement them later.
   1. Add a constructor to `MyGCCopyContext`:
       ```rust
       impl<VM: VMBinding> MyGCCopyContext<VM> {
            pub fn new(mmtk: &'static MMTK<VM>) -> Self {
                Self {
                    plan: &mmtk.plan.downcast_ref::<MyGC<VM>>().unwrap(),
                    mygc: BumpAllocator::new(OpaquePointer::UNINITIALIZED, None, &*mmtk.plan),
                }
            }
        }
       ```
    1. Implement the `WorkerLocal` trait for `MyGCCopyContext`:
        ```rust
        impl<VM: VMBinding> WorkerLocal for MyGCCopyContext<VM> {
            fn init(&mut self, tls: OpaquePointer) {
                CopyContext::init(self, tls);
            }
        }
        ```
   [[Finished code]](/docs/tutorial/code/mygc_semispace/gc_work.rs#L20-L70)
    
6. Add a new public structure, `MyGCProcessEdges`, with the type parameter 
`<VM:VMBinding>`. It will hold an instance of `ProcessEdgesBase` and 
`MyGC`. This is the core part for tracing objects in the `MyGC` plan:
    ```rust
    pub struct MyGCProcessEdges<VM: VMBinding> {
        // Holds a reference to the current plan (Note this will be used in the tracing fast path,
        // and we should not use &dyn Plan here for performance)
        plan: &'static MyGC<VM>,
        base: ProcessEdgesBase<MyGCProcessEdges<VM>>,
    }
    ```
7. Add a new implementations block 
`impl<VM:VMBinding> ProcessEdgesWork for MyGCProcessEdges<VM>`.
   1. Similarly to before, set `ProcessEdgesWork`'s associate type `VM` to 
   the type parameter of `MyGCProcessEdges`, `VM`: `type VM:VM`.
   2. Add a new method, `new`.
       ```rust
        fn new(edges: Vec<Address>, _roots: bool, mmtk: &'static MMTK<VM>) -> Self {
            let base = ProcessEdgesBase::new(edges, mmtk);
            let plan = base.plan().downcast_ref::<MyGC<VM>>().unwrap();
            Self { base, plan }
        }
      ```

8. Now that they've been added, you should import `MyGCCopyContext` and
`MyGCProcessEdges` into `global.rs`, which we will be working in for the
next few steps. [[Finished code]](/docs/tutorial/code/mygc_semispace/global.rs#L1)

9. In `create_worker_local()` in `impl Plan for MyGC`, create an instance of `MyGCCopyContext`:
    ```rust
    fn create_worker_local(
        &self,
        tls: OpaquePointer,
        mmtk: &'static MMTK<Self::VM>,
    ) -> GCWorkerLocalPtr {
        let mut c = MyGCCopyContext::new(mmtk);
        c.init(tls);
        GCWorkerLocalPtr::new(c)
    }
    ```
   
10. `NoCopy` is now no longer needed. Remove it from the import statement block.

11. For the next step, import `crate::scheduler::gc_work::*;`, and modify the
line importing `MMTK` scheduler to read `use crate::scheduler::*;`.
[[Finished code]](/docs/tutorial/code/mygc_semispace/global.rs#L13)

12. Add a new method to `Plan for MyGC`, `schedule_collection()`. This function 
runs when a collection is triggered. It schedules GC work for the plan, i.e.,
it stops all mutators, runs the
scheduler's prepare stage and resumes the mutators. The `StopMutators` work
will invoke code from the bindings to scan threads and other roots, and those scanning work
will further push work for a transitive closure.
    ```rust
    fn schedule_collection(&'static self, scheduler:&MMTkScheduler<VM>) {
        self.base().set_collection_kind();
        self.base().set_gc_status(GcStatus::GcPrepare);
        scheduler.work_buckets[WorkBucketStage::Unconstrained]
            .add(StopMutators::<MyGCProcessEdges<VM>>::new());
        scheduler.work_buckets[WorkBucketStage::Prepare].add(Prepare::<Self, MyGCCopyContext<VM>>::new(self));
        scheduler.work_buckets[WorkBucketStage::Release].add(Release::<Self, MyGCCopyContext<VM>>::new(self));
        scheduler.set_finalizer(Some(EndOfGC));
    }
    ```

## Prepare for collection

The collector has a number of steps it needs to perform before each collection.
We'll add these now.

1. First, fill in some more of the skeleton functions we added to the 
`CopyContext` (in `gc_work.rs`) earlier:
   1. In `prepare()`, rebind the allocator to the tospace using the function
   `self.mygc.rebind(Some(self.plan.tospace()))`.
   2. In `alloc_copy()`, call the allocator's `alloc` function. Above the function, 
   use an inline attribute (`#[inline(always)]`) to tell the Rust compiler 
   to always inline the function. 
   [[Finished code]](/docs/tutorial/code/mygc_semispace/gc_work.rs#L29-L44)
2. In `global.rs`, find the method `prepare`. Delete the `unreachable!()` 
call, and add the following code:
    ```rust
    self.common.prepare(tls, true);
    self.hi
       .store(!self.hi.load(Ordering::SeqCst), Ordering::SeqCst);
    let hi = self.hi.load(Ordering::SeqCst); 
    self.copyspace0.prepare(hi);
    self.copyspace1.prepare(!hi);
    ```
   This function is called at the start of a collection. It prepares the two 
   spaces in the common plan, flips the definitions for which space is 'to' 
   and which is 'from', then prepares the copyspaces with the new definition.
3. Going back to `mutator.rs`, create a new function called 
`mygc_mutator_prepare(_mutator: &mut Mutator <MyGC<VM>>, _tls: OpaquePointer,)`. 
This function will be called at the preparation stage of a collection 
(at the start of a collection) for each mutator. Its body can stay empty, as 
there aren't any preparation steps for the mutator in this GC.
4. In `create_mygc_mutator()`, find the field `prep_func` and change it from
`mygc_mutator_noop()` to `mygc_mutator_prepare()`.


## Scan objects

Next, we'll add the code to allow the plan to collect garbage - filling out 
functions for work packets.

1. In `gc_work.rs`, add a new method to `ProcessEdgesWork for MyGCProcessEdges`,
`trace_object(&mut self, object: ObjectReference)`.
   1. This method should return an ObjectReference, and use the 
   inline attribute.
   2. Check if the object passed into the function is null 
   (`object.is_null()`). If it is, return the object.
   3. Check if the object is in the tospace 
   (`self.plan().tospace().in_space(object)`). If it is, call `trace_object` 
   through the tospace to check if the object is alive, and return the result:
       ```rust
        #[inline]
        fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
            if object.is_null() {
                return object;
            }
            if self.mygc().tospace().in_space(object) {
                self.mygc().tospace().trace_object::<Self, MyGCCopyContext<VM>>(
                    self,
                    object,
                    super::global::ALLOC_MyGC,
                    unsafe { self.worker().local::<MyGCCopyContext<VM>>() },
                )
            } else if self.mygc().fromspace().in_space(object) {
                self.mygc().fromspace().trace_object::<Self, MyGCCopyContext<VM>>(
                    self,
                    object,
                    super::global::ALLOC_MyGC,
                    unsafe { self.worker().local::<MyGCCopyContext<VM>>() },
                )
            } else {
                self.mygc().common.trace_object::<Self, MyGCCopyContext<VM>>(self, object)
            }
        }
       ```
   4. If it is not in the tospace, check if the object is in the fromspace 
   and return the result of the fromspace's `trace_object` if it is.
   5. If it is in neither space, forward the call to the common space and let the common space to handle
   object tracing in its spaces (e.g. immortal or large object space):
   `self.mygc().common.trace_object::<Self, MyGCCopyContext<VM>>(self, object)`.

   [[Finished code (step 1)]](/docs/tutorial/code/mygc_semispace/gc_work.rs#L92-L113)
2. Add two new implementation blocks, `Deref` and `DerefMut` for 
`MyGCProcessEdges`. These allow `MyGCProcessEdges` to be dereferenced to 
`ProcessEdgesBase`, and allows easy access to fields in `ProcessEdgesBase`.
   ```rust
    impl<VM: VMBinding> Deref for MyGCProcessEdges<VM> {
        type Target = ProcessEdgesBase<Self>;
        #[inline]
        fn deref(&self) -> &Self::Target {
            &self.base
        }
    }

    impl<VM: VMBinding> DerefMut for MyGCProcessEdges<VM> {
        #[inline]
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.base
        }
    }
   ```
    
   [[Finished code (step 2)]](/docs/tutorial/code/mygc_semispace/gc_work.rs#L116-L124)
   
3. To `post_copy()`, in the `CopyContext` implementations block, add 
`forwarding_word::clear_forwarding_bits::<VM>(obj);`. Also, add an 
inline attribute.


## Release and Finalize

Finally, we need to fill out the functions that are, roughly speaking, 
run after each collection.

1. Find the method `release()` in `global.rs`. Replace the 
`unreachable!()` call with the following code:
    ```rust
    self.common.release(tls, true);
    self.fromspace().release();
    ```
    This function is called at the end of a collection. It releases the common
    plan spaces and the fromspace.
2. Go back to `mutator.rs`. In `create_mygc_mutator()`, replace 
`mygc_mutator_noop()` in the `release_func` field with `mygc_mutator_release()`.
3. Leave the `release()` function in the `CopyContext` empty. There are no 
release steps for `CopyContext` in this collector.
4. Create a new function called `mygc_mutator_release()` that takes the same 
inputs as the `prepare()` function above. This function will be called at the 
release stage of a collection (at the end of a collection) for each mutator. 
It rebinds the allocator for the `Default` allocation semantics to the new 
tospace. When the mutator threads resume, any new allocations for `Default` 
will then go to the new tospace. The function has the following body:
    ```rust
    let bump_allocator = unsafe {
       mutator
           .allocators
           . get_allocator_mut(
               mutator.config.allocator_mapping[AllocationType::Default]
           )
       }
       .downcast_mut::<BumpAllocator<VM>>()
       .unwrap();
       bump_allocator.rebind(Some(mutator.plan.tospace()));
    ```
5. Delete `mygc_mutator_noop()`. It was a placeholder for the prepare and 
release functions that you have now added, so it is now dead code.
6. Delete `handle_user_collection_request()`. This function was an override of 
a Common plan function to ignore user requested collection for NoGC. Now we 
remove it and allow user requested collection.


You should now have MyGC working and able to collect garbage. All three
 benchmarks should be able to pass now. 

If the benchmarks pass - good job! You have built a functional copying
collector!

If you get particularly stuck, instructions for how to complete this exercise
are available [here](#triplespace-backup-instructions).