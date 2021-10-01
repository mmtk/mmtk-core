# Collection: Implement garbage collection

We need to add a few more things to get garbage collection working. 
Specifically, we need to add a `CopyContext`, which a GC worker uses for 
copying objects, and GC work packets that will be scheduled for a collection.

## CopyContext

At the moment, none of the files in the plan are suited for garbage collection 
operations. So, we need to add a new file to hold the `CopyContext` and other 
structures and functions that will give the collector proper functionality.

Make a new file under `mygc`, called `gc_work.rs`. 
In `mod.rs`, import `gc_work` as a module by adding the line `mod gc_work`.
In `gc_work.rs`, add the following import statements:
```rust
{{#include ../../../code/mygc_semispace/gc_work.rs:imports}}
```

Add a new structure, `MyGCCopyContext`, with the type parameter 
`VM: VMBinding`. It should have the fields `plan: &'static MyGC<VM>`
and `mygc: BumpAllocator`.

```rust
{{#include ../../../code/mygc_semispace/gc_work.rs:mygc_copy_context}}
```
   
Create an implementation block - 
`impl<VM: VMBinding> CopyContext for MyGCCopyContext<VM>`.
Define the associate type `VM` for `CopyContext` as the VMBinding type 
given to the class as `VM`: `type VM = VM`. 

Add the following skeleton functions (taken from `plan/global.rs`):

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
fn alloc_copy(
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

In `init()`, set the `tls` variable in the held instance of `mygc` to the one 
passed to the function. In `constraints()`, return a reference of 
`MYGC_CONSTRAINTS`.

```rust
{{#include ../../../code/mygc_semispace/gc_work.rs:copycontext_constraints_init}}
```

We just leave the rest of the functions empty for now and will implement them 
later.

Add a constructor to `MyGCCopyContext` and implement the `WorkerLocal` trait 
for `MyGCCopyContext`.

```rust
{{#include ../../../code/mygc_semispace/gc_work.rs:constructor_and_workerlocal}}
```

## MyGCProcessEdges
    
Add a new public structure, `MyGCProcessEdges`, with the type parameter 
`<VM:VMBinding>`. It will hold an instance of `ProcessEdgesBase` and 
`MyGC`. This is the core part for tracing objects in the `MyGC` plan.

```rust
{{#include ../../../code/mygc_semispace/gc_work.rs:mygc_process_edges}}
```

Add a new implementations block 
`impl<VM:VMBinding> ProcessEdgesWork for MyGCProcessEdges<VM>`.
Similarly to before, set `ProcessEdgesWork`'s associate type `VM` to 
the type parameter of `MyGCProcessEdges`, `VM`: `type VM:VM`.
Add a new constructor, `new()`.

```rust
{{#include ../../../code/mygc_semispace/gc_work.rs:mygc_process_edges_new}}
```

## Introduce collection to MyGC plan

Now that they've been added, you should import `MyGCCopyContext` and
`MyGCProcessEdges` into `mygc/global.rs`, which we will be working in for the
next few steps. 

```rust
{{#include ../../../code/mygc_semispace/global.rs:imports_gc_work}}
```

In `create_worker_local()` in `impl Plan for MyGC`, create an instance of 
`MyGCCopyContext`.

```rust
{{#include ../../../code/mygc_semispace/global.rs:create_worker_local}}
```

`NoCopy` is now no longer needed. Remove it from the import statement block. 
For the next step, import `crate::scheduler::gc_work::*;`, and modify the
line importing `MMTK` scheduler to read `use crate::scheduler::*;`.

Add a new method to `Plan for MyGC`, `schedule_collection()`. This function 
runs when a collection is triggered. It schedules GC work for the plan, i.e.,
it stops all mutators, runs the
scheduler's prepare stage and resumes the mutators. The `StopMutators` work
will invoke code from the bindings to scan threads and other roots, and those 
scanning work will further push work for a transitive closure.

```rust
{{#include ../../../code/mygc_semispace/global.rs:schedule_collection}}
```

Delete `handle_user_collection_request()`. This function was an override of 
a Common plan function to ignore user requested collection for NoGC. Now we 
remove it and allow user requested collection.

## Prepare for collection

The collector has a number of steps it needs to perform before each collection.
We'll add these now.

### Prepare plan

In `mygc/global.rs`, find the method `prepare`. Delete the `unreachable!()` 
call, and add the following code:

```rust
{{#include ../../../code/mygc_semispace/global.rs:prepare}}
```

This function is called at the start of a collection. It prepares the two 
spaces in the common plan, flips the definitions for which space is 'to' 
and which is 'from', then prepares the copyspaces with the new definition.

### Prepare CopyContext

First, fill in some more of the skeleton functions we added to the 
`CopyContext` (in `gc_work.rs`) earlier.
In `prepare()`, rebind the allocator to the tospace using the function
   `self.mygc.rebind(Some(self.plan.tospace()))`.

```rust
{{#include ../../../code/mygc_semispace/gc_work.rs:copycontext_prepare}}
```

### Prepare mutator

Going back to `mutator.rs`, create a new function called 
`mygc_mutator_prepare(_mutator: &mut Mutator <MyGC<VM>>, _tls: OpaquePointer,)`. 
This function will be called at the preparation stage of a collection 
(at the start of a collection) for each mutator. Its body can stay empty, as 
there aren't any preparation steps for the mutator in this GC.
In `create_mygc_mutator()`, find the field `prep_func` and change it from
`mygc_mutator_noop()` to `mygc_mutator_prepare()`.


## Scan objects

Next, we'll add the code to allow the plan to collect garbage - filling out 
functions for work packets.

In `gc_work.rs`, add a new method to `ProcessEdgesWork for MyGCProcessEdges`,
`trace_object(&mut self, object: ObjectReference)`.
This method should return an ObjectReference, and use the 
inline attribute.
Check if the object passed into the function is null 
(`object.is_null()`). If it is, return the object.
Check if which space the object is in, and forward the call to the 
policy-specific object tracing code. If it is in neither space, forward the 
call to the common space and let the common space to handle object tracing in 
its spaces (e.g. immortal or large object space):

```rust
{{#include ../../../code/mygc_semispace/gc_work.rs:trace_object}}
```

Add two new implementation blocks, `Deref` and `DerefMut` for 
`MyGCProcessEdges`. These allow `MyGCProcessEdges` to be dereferenced to 
`ProcessEdgesBase`, and allows easy access to fields in `ProcessEdgesBase`.

```rust
{{#include ../../../code/mygc_semispace/gc_work.rs:deref}}
```

## Copying objects

Go back to the `MyGCopyContext` in `gc_work.rs`. 
In `alloc_copy()`, call the allocator's `alloc` function. Above the function, 
   use an inline attribute (`#[inline(always)]`) to tell the Rust compiler 
   to always inline the function.

```rust
{{#include ../../../code/mygc_semispace/gc_work.rs:copycontext_alloc_copy}}
```

To `post_copy()`, in the `CopyContext` implementations block, add 
`forwarding_word::clear_forwarding_bits::<VM>(obj);`. Also, add an 
inline attribute.

```rust
{{#include ../../../code/mygc_semispace/gc_work.rs:copycontext_post_copy}}
```

## Release

Finally, we need to fill out the functions that are, roughly speaking, 
run after each collection.

### Release in plan

Find the method `release()` in `mygc/global.rs`. Replace the 
`unreachable!()` call with the following code.

```rust
{{#include ../../../code/mygc_semispace/global.rs:release}}
```

This function is called at the end of a collection. It calls the release 
routines for the common plan spaces and the fromspace.

### Release in mutator

Go back to `mutator.rs`. In `create_mygc_mutator()`, replace 
`mygc_mutator_noop()` in the `release_func` field with `mygc_mutator_release()`.
Leave the `release()` function in the `CopyContext` empty. There are no 
release steps for `CopyContext` in this collector.

Create a new function called `mygc_mutator_release()` that takes the same 
inputs as the `prepare()` function above. This function will be called at the 
release stage of a collection (at the end of a collection) for each mutator. 
It rebinds the allocator for the `Default` allocation semantics to the new 
tospace. When the mutator threads resume, any new allocations for `Default` 
will then go to the new tospace.
 
```rust
{{#include ../../../code/mygc_semispace/mutator.rs:release}}
```

Delete `mygc_mutator_noop()`. It was a placeholder for the prepare and 
release functions that you have now added, so it is now dead code.

## Summary

You should now have MyGC working and able to collect garbage. All three
benchmarks should be able to pass now. 

If the benchmarks pass - good job! You have built a functional copying
collector!

If you get particularly stuck, the code for the completed `MyGC` plan
is available [here](https://github.com/mmtk/mmtk-core/tree/master/docs/tutorial/code/mygc_semispace).