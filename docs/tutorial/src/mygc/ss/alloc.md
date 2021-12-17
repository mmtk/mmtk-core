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
{{#include ../../../code/mygc_semispace/global.rs:constraints}}
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
{{#include ../../../code/mygc_semispace/global.rs:imports_no_gc_work}}
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
{{#include ../../../code/mygc_semispace/global.rs:plan_def}}
```

### Implement the Plan trait for MyGC

#### Constructor

Change `fn new()`. This section initialises and prepares the objects in MyGC 
that you just defined.

   1. Delete the definition of `mygc_space`. 
   Instead, we will define the two copyspaces here.
   2. Define one of the copyspaces by adding the following code: 
```rust
{{#include ../../../code/mygc_semispace/global.rs:copyspace_new}}
```

   3. Create another copyspace, called `copyspace1`, defining it as a fromspace 
   instead of a tospace. (Hint: the definitions for 
   copyspaces are in `src/policy/copyspace.rs`.) 
   4. Finally, replace the old MyGC initializer.
```rust
{{#include ../../../code/mygc_semispace/global.rs:plan_new}}
```

#### Initializer

Find `gc_init()`. Change it to initialise the common plan and the two 
copyspaces, rather than the base plan and mygc_space. The contents of the 
initializer calls are identical.

```rust
{{#include ../../../code/mygc_semispace/global.rs:gc_init}}
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

```rust
{{#include ../../../code/mygc_semispace/global.rs:plan_space_access}}
```

#### Other methods in the Plan trait

The trait `Plan` requires a `common()` method that should return a 
reference to the common plan. Implement this method now.

```rust
{{#include ../../../code/mygc_semispace/global.rs:plan_common}}
```

Find the helper method `base` and change it so that it calls the 
base plan *through* the common plan.

```rust
{{#include ../../../code/mygc_semispace/global.rs:plan_base}}
```

The trait `Plan` requires `collection_required()` method to know when
we should trigger a collection. We can just use the implementation
in the `BasePlan`.

```rust
{{#include ../../../code/mygc_semispace/global.rs:collection_required}}
```

Find the method `get_pages_used`. Replace the current body with 
`self.tospace().reserved_pages() + self.common.get_pages_used()`, to 
correctly count the pages contained in the tospace and the common plan 
spaces (which will be explained later).

```rust
{{#include ../../../code/mygc_semispace/global.rs:plan_get_pages_used}}
```

Add and override the following helper function:

```rust
{{#include ../../../code/mygc_semispace/global.rs:plan_get_collection_reserve}}
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
{{#include ../../../code/mygc_semispace/mutator.rs:imports}}
```

### Prepare and release mutator

First, we define two functions that will be used before and after GCs to prepare and
release mutators.

For our semispace GC, we do not need to do anything special for a mutator before a GC.
So we leave the implementation of the prepare function as empty.

```rust
{{#include ../../../code/mygc_semispace/mutator.rs:mutator_prepare}}
```

As we will flip the from space and the to space in each GC, we will have to rebind
each mutator after a GC to the new to space. So we do the rebind in the release function.

```rust
{{#include ../../../code/mygc_semispace/mutator.rs:mutator_release}}
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
{{#include ../../../code/mygc_semispace/mutator.rs:allocator_mapping}}
```

### Create mutator config

We need to provide an implementation of the mutator configuration in the `Plan` trait.
Here we implement `create_mutator_config()` from the functions and mappings we defined in
the last few steps.

The implementation of `create_mutator_config()` is quite straight forward. We have defined
the prepare and the release functions, and we have defined the allocator mapping. We simply
use them in the mutator config.

```rust
{{#include ../../../code/mygc_semispace/global.rs:create_mutator_config}}
```

One thing that is worth noting is the space mapping in the mutator config.
the space allocation is formatted as a list of tuples. Similar to `create_allocator_mapping()`,
we use `create_space_mapping()` to create a default space mapping. Then we bind
the first bump pointer allocator (`BumpPointer(0)`) with `tospace`.

```rust
{{#include ../../../code/mygc_semispace/global.rs:create_mutator_config_space_mapping}}
```

### Under the hood (common and base spaces)

Apart from the to and the from space used in our semispace plan, any MMTk plan includes
a few base spaces and common spaces. The allocator mapping and space mapping for those spaces
are handled by the `create_space_mapping` and `create_allocator_mapping` calls that we used above.
Thus the plan implementor does not need to worry about mapping for those spaces.

To be specific, those spaces include:

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