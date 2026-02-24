# eBPF-based tracing tools

## Notes for MMTk developers

Please open pull requests if you develop new tools that others might find useful.
When you add new tools, please update this documentation.
If you change MMTk internals that the tracing tools depend on (such as the
definition of `enum WorkBucketStage`), please update the scripts accordingly.

## Notes for MMTk users

Since some of the tools depend on the MMTk internals, please use the tools
shipped with the MMTk release you use.

## Tracepoints

Currently, the core provides the following tracepoints.

-   `mmtk:collection_initialized()`: All GC worker threads are spawn
-   `mmtk:prepare_to_fork()`: The VM requests MMTk core to prepare for calling `fork()`.
-   `mmtk:after_fork()`: The VM notifies MMTk core it has finished calling `fork()`.
-   `mmtk:goal_set(goal: int)`: GC workers have started working on a goal.
-   `mmtk:goal_complete(goal: int)`: GC workers have fihisned working on a goal.
-   `mmtk:harness_begin()`: the timing iteration of a benchmark begins
-   `mmtk:harness_end()`: the timing iteration of a benchmark ends
-   `mmtk:gcworker_run()`: a GC worker thread enters its work loop
-   `mmtk:gcworker_exit()`: a GC worker thread exits its work loop
-   `mmtk:gc_start()`: a collection epoch starts
-   `mmtk:gc_end()`: a collection epoch ends
-   `mmtk:gen_full_heap(is_full_heap: bool)`: the generational plan has determined whether the current
    GC is a full heap GC.  Only executed if the plan is generational.
-   `mmtk:immix_defrag(is_defrag_gc: bool)`: the Immix-based plan has determined whether the current
    GC is a defrag GC.  Only executed if the plan is Immix-based (i.e. Immix, GenImmix and
    StickyImmix).  Will not be executed during nursery GCs (for GenImmix and StickyImmix).
-   `mmtk:roots(kind: int, len: int)`: reporing roots to mmtk-core during root scanning.  `kind` can
    be 0, 1 or 2 for normal roots, pinning roots and transitively pinning roots, respectively.
    `len` is the number of slots or nodes reported.
-   `mmtk:process_root_nodes(num_roots: int, num_enqueued_nodes: int)`: a ProcessRootNodes work
    packet which processes root edges represented as object references to the target objects.
-   `mmtk:process_slots(num_slots: int, is_roots: bool)`: an invocation of the `process_slots`
    method. The first argument is the number of slots to be processed, and the second argument is
    whether these slots are root slots.
-   `mmtk:scan_objects(total_objects: int, scan_and_trace: int)`: an invocation of the
    `ScanObjectsWork::do_work_common` method.  `total_objects` is the total number of objects in the
    work packet, and `scan_and_trace` is the number of objects scanned using the
    `Scanning::scan_object_and_trace_edges` method. Other objects are scanned using
    `Scanning::scan_object`.
-   `mmtk:sweep_chunk(allocated_blocks: int)`: an execution of the `SweepChunk` work packet (for
    both `MarkSweepSpace` and `ImmixSpace`).  `allocated_blocks` is the number of allocated blocks
    in the chunk processed by the work packet.
-   `mmtk:bucket_opened(id: int)`: a work bucket opened. The first argument is the numerical
    representation of `enum WorkBucketStage`.
-   `mmtk:work_poll()`: a work packet is to be polled.
-   `mmtk:work(type_name: char *, type_name_len: int)`: a work packet was just executed. The first
    argument is points to the string of the Rust type name of the work packet, and the second
    argument is the length of the string.
-   `mmtk:alloc_slow_once_start()`: the allocation slow path starts.
-   `mmtk:alloc_slow_once_end()`: the allocation slow path ends.
-   `mmtk:plan_end_of_gc_begin()`: before executing `Plan::end_of_gc`.
-   `mmtk:plan_end_of_gc_end()`: after executing `Plan::end_of_gc`.

## Tracing tools

Each sub-directory contains a set of scripts.

-   `performance`: Print various GC-related statistics, such as the distribution of time spent in
    allocation slow path, the time spent in each GC stages, and the distribution of the
    `ProcessEdgesWork` packet sizes.
-   `timeline`: Record the start and end time of each GC and each work packet, and visualize them on
    a timeline in Perfetto UI.

## Attribution

If used for research, please cite the following publication.

```bibtex
@inproceedings{DBLP:conf/pppj/HuangBC23,
  author       = {Claire Huang and
                  Stephen M. Blackburn and
                  Zixian Cai},
  editor       = {Rodrigo Bruno and
                  Eliot Moss},
  title        = {Improving Garbage Collection Observability with Performance Tracing},
  booktitle    = {Proceedings of the 20th {ACM} {SIGPLAN} International Conference on
                  Managed Programming Languages and Runtimes, {MPLR} 2023, Cascais,
                  Portugal, 22 October 2023},
  pages        = {85--99},
  publisher    = {{ACM}},
  year         = {2023},
  url          = {https://doi.org/10.1145/3617651.3622986},
  doi          = {10.1145/3617651.3622986},
  timestamp    = {Mon, 23 Oct 2023 17:57:18 +0200},
  biburl       = {https://dblp.org/rec/conf/pppj/HuangBC23.bib},
  bibsource    = {dblp computer science bibliography, https://dblp.org}
}
```
