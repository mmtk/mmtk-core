# Allocation: Add copyspaces

We will now change your MyGC plan from one that cannot collect garbage
into one that implements the semispace algorithm. The first step of this
is to add the two copyspaces, and allow collectors to allocate memory 
into them. This involves adding two copyspaces, the code to properly initialise 
and prepare the new spaces, and a copy context.

## Change the plan constraints

Firstly, change the plan constraints. Some of these constraints are not used 
at the moment, but it's good to set them properly regardless.

Look in `plan/plan_constraints.rs`. `PlanConstraints` lists all the possible
options for plan-specific constraints. At the moment, `MYGC_CONSTRAINTS` in 
`mygc/global.rs` should be using the default value for `PlanConstraints`. 
We will make the following changes:

1. Initialize `gc_header_bits` to 2. We reserve 2 bits in the header for GC use.
1. Initialize `moves_objects` to `true`.
1. Initialize `num_specialized_scans` to 1.

Finished code (step 1-3):
```
{{#include ../../code/mygc_semispace/global.rs:constraints}}
```

## Change the plan implementation

Next, in `mygc/global.rs`, replace the old immortal (nogc) space with two 
copyspaces.

### Imports

To the import statement block:

   1. Replace `crate::plan::global::{BasePlan, NoCopy};` with 
   `use crate::plan::global::BasePlan;`. This collector is going to use 
   copying, so there's no point to importing NoCopy any more.
   2. Add `use crate::plan::global::CommonPlan;`. Semispace uses the common
   plan, which includes an immortal space and a large object space, rather 
   than the base plan. Any garbage collected plan should use `CommonPlan`.
   3. Add `use std::sync::atomic::{AtomicBool, Ordering};`. These are going 
   to be used to store an indicator of which copyspace is the tospace.
   4. Delete `#[allow(unused_imports)]`.

Finished code (step 1):
```rust
{{#include ../../code/mygc_semispace/global.rs:imports_no_gc_work}}
```

### Struct MyGC

Change `pub struct MyGC<VM: VMBinding>` to add new instance variables.

   1. Delete the existing fields in the constructor.
   2. Add `pub hi: AtomicBool,`. This is a thread-safe bool, indicating which 
   copyspace is the tospace.
   3. Add `pub copyspace0: CopySpace<VM>,` 
   and `pub copyspace1: CopySpace<VM>,`. These are the two copyspaces.
   4. Add `pub common: CommonPlan<VM>,`.
    This holds an instance of the common plan.

Finished code (step 2):
```rust
{{#include ../../code/mygc_semispace/global.rs:plan_def}}
```

Note that `MyGC` now also derives `PlanTraceObject` besides `HasSpaces`, and we
have attributes on some fields. These attributes tell MMTk's macros how to
generate code to visit each space of this plan as well as trace objects in this
plan.  Although there are other approaches that you can implement object
tracing, in this tutorial we use the macros, as it is the simplest.  Make sure
you import the macros. We will discuss on what those attributes mean in later
sections.

```rust
use mmtk_macros::{HasSpaces, PlanTraceObject};
```

### Implement the Plan trait for MyGC

#### Constructor

Change `fn new()`. This section initialises and prepares the objects in MyGC 
that you just defined.

   1. Delete the definition of `mygc_space`. 
   Instead, we will define the two copyspaces here.
   2. Define one of the copyspaces by adding the following code: 
```rust
{{#include ../../code/mygc_semispace/global.rs:copyspace_new}}
```

   3. Create another copyspace, called `copyspace1`, defining it as a fromspace 
   instead of a tospace. (Hint: the definitions for 
   copyspaces are in `src/policy/copyspace.rs`.) 
   4. Finally, replace the old MyGC initializer.
```rust
{{#include ../../code/mygc_semispace/global.rs:plan_new}}
```

### Access MyGC spaces

Add a new section of methods for MyGC:

```rust
impl<VM: VMBinding> MyGC<VM> {
}
```

To this, add two helper methods, `tospace(&self)` 
and `fromspace(&self)`. They both have return type `&CopySpace<VM>`, 
and return a reference to the tospace and fromspace respectively. 
`tospace()` (see below) returns a reference to the tospace, 
and `fromspace()` returns a reference to the fromspace.

We also add another two helper methods to get `tospace_mut(&mut self)`
and `fromspace_mut(&mut self)`. Those will be used later when we implement
collection for our GC plan.

```rust
{{#include ../../code/mygc_semispace/global.rs:plan_space_access}}
```

#### Other methods in the Plan trait

The trait `Plan` requires a `common()` method that should return a 
reference to the common plan. Implement this method now.

```rust
{{#include ../../code/mygc_semispace/global.rs:plan_common}}
```

Find the helper method `base` and change it so that it calls the 
base plan *through* the common plan.

```rust
{{#include ../../code/mygc_semispace/global.rs:plan_base}}
```

The trait `Plan` requires `collection_required()` method to know when
we should trigger a collection. We can just use the implementation
in the `BasePlan`.

```rust
{{#include ../../code/mygc_semispace/global.rs:collection_required}}
```

Find the method `get_pages_used`. Replace the current body with 
`self.tospace().reserved_pages() + self.common.get_pages_used()`, to 
correctly count the pages contained in the tospace and the common plan 
spaces (which will be explained later).

```rust
{{#include ../../code/mygc_semispace/global.rs:plan_get_used_pages}}
```

Add and override the following helper function:

```rust
{{#include ../../code/mygc_semispace/global.rs:plan_get_collection_reserve}}
```

## Change the mutator definition

Next, we need to change the mutator, in `mutator.rs`, to allocate to the 
tospace, and to the two spaces controlled by the common plan. 

### Imports

Change the following import statements:
   1. Add `use super::MyGC;`.
   2. Add `use crate::util::alloc::BumpAllocator;`.
   3. Delete `use crate::plan::mygc::MyGC;`.

```rust
{{#include ../../code/mygc_semispace/mutator.rs:imports}}
```

### Allocator mapping

In `lazy_static!`, make the following changes to `ALLOCATOR_MAPPING`, 
which maps the required allocation semantics to the corresponding allocators. 
For example, for `Default`, we allocate using the first bump pointer allocator 
(`BumpPointer(0)`):
   1. Define a `ReservedAllocators` instance to declare that we need one bump allocator.
   2. Map the common plan allocators using `create_allocator_mapping`.
   3. Map `Default` to `BumpPointer(0)`.

```rust
{{#include ../../code/mygc_semispace/mutator.rs:allocator_mapping}}
```

### Space mapping

Next, in `create_mygc_mutator`, change which allocator is allocated to what 
space in `space_mapping`. Note that the space allocation is formatted as a list 
of tuples. For example, the first bump pointer allocator (`BumpPointer(0)`) is 
bound with `tospace`.

Downcast the dynamic `Plan` type to `MyGC` so we can access specific spaces in 
`MyGC`.

```rust
{{#include ../../code/mygc_semispace/mutator.rs:plan_downcast}}
```

Then, use `mygc` to access the spaces in `MyGC`.

   1. `BumpPointer(0)` should map to the tospace.
   2. Other common plan allocators should be mapped using `create_space_mapping`.
   3. None of the above should be dereferenced (ie, they should not have
   the `&` prefix).

```rust
{{#include ../../code/mygc_semispace/mutator.rs:space_mapping}}
```
     
The `create_space_mapping` and `create_allocator_mapping` call that have appeared all
of a sudden in these past 2 steps, are parts of the MMTk common plan
itself. They are used to construct allocator-space mappings for the spaces defined
by the common plan:

 1. The immortal space is used for objects that the virtual machine or a 
 library never expects to die.
 2. The large object space is needed because MMTk handles particularly large 
 objects differently to normal objects, as the space overhead of copying 
 large objects is very high. Instead, this space is used by a free list 
 allocator in the common plan to avoid having to copy them. 
 3. The read-only space is used to store all the immutable objects.
 4. The code spaces are used for VM generated code objects.

With this, you should have the allocation working, but not garbage collection. 
Try building again. If you run HelloWorld or Fannkunchredux, they should
work. DaCapo's lusearch should fail, as it requires garbage to be collected. 
