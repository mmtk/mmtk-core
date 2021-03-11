# Allocation: Add copyspaces

We will now change your MyGC plan from one that cannot collect garbage
into one that implements the semispace algorithm. The first step of this
is to add the two copyspaces, and allow collectors to allocate memory 
into them. This involves adding two copyspaces, the code to properly initialise 
and prepare the new spaces, and a copy context.


Firstly, change the plan constraints. Some of these constraints are not used 
at the moment, but it's good to set them properly regardless.
1. Look in `plan/plan_constraints.rs`. `PlanConstraints` lists all the possible
options for plan-specific constraints. At the moment, `MYGC_CONSTRAINTS` in `mygc/global.rs` should be using
the default value for `PlanConstraints`. We will make the following changes.
1. Initialize `gc_header_bits` to 2. We reserve 2 bits in the header for GC use.
1. Initialize `moves_objects` to `true`.
1. Initialize `num_specialized_scans` to 1.
[[Finished code (step 1-4)]](/docs/tutorial/code/mygc_semispace/global.rs#L45-L51)

Next, in `global.rs`, replace the old immortal (nogc) space with two copyspaces.
1. To the import statement block:
   1. Replace `crate::plan::global::{BasePlan, NoCopy};` with 
   `use crate::plan::global::BasePlan;`. This collector is going to use 
   copying, so there's no point to importing NoCopy anymore.
   2. Add `use crate::plan::global::CommonPlan;`. Semispace uses the common
   plan, which includes an immortal space and a large object space, rather 
   than the base plan. Any garbage collected plan should use `CommonPlan`.
   3. Add `use std::sync::atomic::{AtomicBool, Ordering};`. These are going 
   to be used to store an indicator of which copyspace is the tospace.
   4. Delete `#[allow(unused_imports)]`.
   
   [[Finished code (step 1)]](/docs/tutorial/code/mygc_semispace/global.rs#L1)
   
2. Change `pub struct MyGC<VM: VMBinding>` to add new instance variables.
   1. Delete the existing fields in the constructor.
   2. Add `pub hi: AtomicBool,`. This is a thread-safe bool, indicating which 
   copyspace is the tospace.
   3. Add `pub copyspace0: CopySpace<VM>,` 
   and `pub copyspace1: CopySpace<VM>,`. These are the two copyspaces.
   4. Add `pub common: CommonPlan<VM>,`.
    This holds an instance of the common plan.

   [[Finished code (step 2)]](/docs/tutorial/code/mygc_semispace/global.rs#L36-L41)
  
3. Change `impl<VM: VMBinding> Plan for MyGC<VM>`.
This section initialises and prepares the objects in MyGC that you just defined.
   1. Delete the definition of `mygc_space`. 
   Instead, we will define the two copyspaces here.
   2. Define one of the copyspaces by adding the following code: 
       ```rust
        let copyspace0 = CopySpace::new(
             "copyspace0",
             false,
             true,
             VMRequest::discontiguous(),
             vm_map,
             mmapper,
             &mut heap,
         );
       ```
   3. Create another copyspace, called `copyspace1`, defining it as a fromspace 
   instead of a tospace. (Hint: the definitions for 
   copyspaces are in `src/policy/copyspace.rs`.) 
   4. Finally, replace the old MyGC initializer with the following:
       ```rust
        MyGC {
            hi: AtomicBool::new(false),
            copyspace0,
            copyspace1,
            common: CommonPlan::new(vm_map, mmapper, options, heap, &MYGC_CONSTRAINTS),
        }
       ```

   [[Finished code (step 3-4)]](/docs/tutorial/code/mygc_semispace/global.rs#L147-L168)
   
4. There are a few more functions to add to `Plan for MyGC` next.
     1. Find `gc_init()`. Change it to initialise the common plan and the two 
     copyspaces, rather than the base plan and mygc_space. The contents of the 
     initializer calls are identical.
     
     [[Finished code (step 4 i)]](/docs/tutorial/code/mygc_semispace/global.rs#L71-L80)
     
     2. The trait `Plan` requires a `common()` method that should return a 
     reference to the common plan. Implement this method now.
         ```rust
         fn common(&self) -> &CommonPlan<VM> {
           &self.common
         }
         ```
      3. Find the helper method `base` and change it so that it calls the 
      base plan *through* the common plan.
          ```rust
          fn base(&self) -> &BasePlan<VM> {
            &self.common.base
          }
         ```
      4. Find the method `get_pages_used`. Replace the current body with 
      `self.tospace().reserved_pages() + self.common.get_pages_used()`, to 
      correctly count the pages contained in the tospace and the common plan 
      spaces (which will be explained later).
      5. Also add the following helper function:
         ```rust
         fn get_collection_reserve(&self) -> usize {
             self.tospace().reserved_pages()
         }
         ``` 
      
      [[Finished code (step 4 ii-iv)]](/docs/tutorial/code/mygc_semispace/global.rs#L115-L133)
      
5. Add a new section of methods for MyGC:
    ```rust
    impl<VM: VMBinding> MyGC<VM> {
    }
    ```
   1. To this, add two helper methods, `tospace(&self)` 
   and `fromspace(&self)`. They both have return type `&CopySpace<VM>`, 
   and return a reference to the tospace and fromspace respectively. 
   `tospace()` (see below) returns a reference to the tospace, 
   and `fromspace()` returns a reference to the fromspace.
       ```rust
       pub fn tospace(&self) -> &CopySpace<VM> {
         if self.hi.load(Ordering::SeqCst) {
             &self.copyspace1
         } else {
             &self.copyspace0
         }
       }

        pub fn fromspace(&self) -> &CopySpace<VM> {
            if self.hi.load(Ordering::SeqCst) {
                &self.copyspace0
            } else {
                &self.copyspace1
            }
        }
      ```
   
   [[Finished code (step 5)]](/docs/tutorial/code/mygc_semispace/global.rs#L171-L185)

          
Next, we need to change the mutator, in `mutator.rs`, to allocate to the 
tospace, and to the two spaces controlled by the common plan. 
1. Change the following import statements:
   1. Add `use super::MyGC;`.
   2. Add `use crate::util::alloc::BumpAllocator;`.
   3. Delete `use crate::plan::mygc::MyGC;`.
   
   [[Finished code (step 1)]](/docs/tutorial/code/mygc_semispace/mutator.rs#L1)

2. In `lazy_static!`, make the following changes to `ALLOCATOR_MAPPING`, 
which maps the required allocation semantics to the corresponding allocators. 
For example, for `Default`, we allocate using the first bump pointer allocator 
(`BumpPointer(0)`):
   1. Map `Default` to `BumpPointer(0)`.
   2. Map `ReadOnly` to `BumpPointer(1)`.
   3. Map `Los` to `LargeObject(0)`. 
   
   [[Finished code (step 2)]](/docs/tutorial/code/mygc_semispace/mutator.rs#L47-L51)
   
3. Next, in `create_mygc_mutator`, change which allocator is allocated to what 
space in `space_mapping`. Note that the space allocation is formatted as a list 
of tuples. For example, the first bump pointer allocator (`BumpPointer(0)`) is 
bound with `tospace`. 
   1. `BumpPointer(0)` should map to the tospace.
   2. `BumpPointer(1)` should map to `plan.common.get_immortal()`.
   3. `LargeObject(0)` should map to `plan.common.get_los()`.
   4. None of the above should be dereferenced (ie, they should not have 
   the `&` prefix).
   
   [[Finished code (step 3)]](/docs/tutorial/code/mygc_semispace/mutator.rs#L54-L80)
     
There may seem to be 2 extraneous spaces and allocators that have appeared all 
of a sudden in these past 2 steps. These are parts of the MMTk common plan 
itself.
 1. The immortal space is used for objects that the virtual machine or a 
 library never expects to die.
 2. The large object space is needed because MMTk handles particularly large 
 objects differently to normal objects, as the space overhead of copying 
 large objects is very high. Instead, this space is used by a free list 
 allocator in the common plan to avoid having to copy them. 

With this, you should have the allocation working, but not garbage collection. 
Try building again. If you run HelloWorld or Fannkunchredux, they should
work. DaCapo's lusearch should fail, as it requires garbage to be collected. 