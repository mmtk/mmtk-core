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
-   `mmtk:prepare_fork()`: The VM requests MMTk core to prepare for calling `fork()`.
-   `mmtk:after_fork()`: The VM notifies MMTk core it has finished calling `fork()`.
-   `mmtk:goal_set(goal: int)`: GC workers have started working on a goal.
-   `mmtk:goal_complete(goal: int)`: GC workers have fihisned working on a goal.
-   `mmtk:harness_begin()`: the timing iteration of a benchmark begins
-   `mmtk:harness_end()`: the timing iteration of a benchmark ends
-   `mmtk:gcworker_run()`: a GC worker thread enters its work loop
-   `mmtk:gcworker_exit()`: a GC worker thread exits its work loop
-   `mmtk:gc_start()`: a collection epoch starts
-   `mmtk:gc_end()`: a collection epoch ends
-   `mmtk:process_edges(num_edges: int, is_roots: bool)`: a invocation of the `process_edges`
    method. The first argument is the number of edges to be processed, and the second argument is
    whether these edges are root edges.
-   `mmtk:bucket_opened(id: int)`: a work bucket opened. The first argument is the numerical
    representation of `enum WorkBucketStage`.
-   `mmtk:work_poll()`: a work packet is to be polled.
-   `mmtk:work(type_name: char *, type_name_len: int)`: a work packet was just executed. The first
    argument is points to the string of the Rust type name of the work packet, and the second
    argument is the length of the string.
-   `mmtk:alloc_slow_once_start()`: the allocation slow path starts.
-   `mmtk:alloc_slow_once_end()`: the allocation slow path ends.

## Tracing tools

Each sub-directory contains a set of scripts.

-   `performance`: Print various GC-related statistics, such as the distribution of time spent in
    allocation slow path, the time spent in each GC stages, and the distribution of `process_edges`
    packet sizes.
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
