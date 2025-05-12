# Print MMTk Object Information

MMTk provides a utility function to print object information for debugging, `crate::mmtk::mmtk_debug_print_object_info`.
The function is marked as `#[no_mangle]`, making it suitable to be used in a debugger.

The following example shows how to use the function to print MMTk's object metadata before and after a single GC in `rr`.

Set up break points before and after a GC.

```console
(rr) b stop_all_mutators
Breakpoint 1 at 0x767cba8f74e8 (14 locations)
(rr) b resume_mutators
Breakpoint 2 at 0x767cba8f8908
```

When the program stops, call `mmtk_debug_print_object_info`. We might need to
set the language context in the debugger to `C` when we stop in a Rust frame.
Then call `mmtk_debug_print_object_info` with the interested object.

```console
(rr) set language c
(rr) call mmtk_debug_print_object_info(0x200fffed9f0)
immix: marked = false, line marked = false, block state = Unmarked, forwarding bits = 0, forwarding pointer = None, vo bit = true
```
