# MMTk performance tracing

This directory contains tools for printing out various GC-related statistics using eBPF tracing.

## Running

Before running, you should make sure the [bpftrace] command line utility is installed.

[bpftrace]: https://github.com/iovisor/bpftrace

Tracing tools in this directory are to be invoked by a wrapper script `run.py`.

```
usage: run.py [-h] [-b BPFTRACE] -m MMTK [-H] [-p] [-f {text,json}] tool

positional arguments:
  tool                  Name of the bpftrace tool

optional arguments:
  -h, --help            show this help message and exit
  -b BPFTRACE, --bpftrace BPFTRACE
                        Path of the bpftrace executable
  -m MMTK, --mmtk MMTK  Path of the MMTk binary
  -H, --harness         Only collect data for the timing iteration (harness_begin/harness_end)
  -p, --print-script    Print the content of the bpftrace script
  -f {text,json}, --format {text,json}
                        bpftrace output format
```

- `-b`: the path to the `bpftrace` executable. By default, it uses `bpftrace`
executable in your `PATH`. We strongly recommend you use the latest statically
complied `bpftrace` from [upstream](https://github.com/iovisor/bpftrace/releases).
You need to be able to have sudo permission for whichever `bpftrace` you want to use.
- `-m`: the path to a MMTk binary that contains the tracepoints.
This depends on the binding you use.
For the OpenJDK binding, it should be `jdk/lib/server/libmmtk_openjdk.so` under
your build folder.
To check whether the binary contains tracepoints, you can use `readelf -n`.
You should see a bunch of `stapsdt` notes with `mmtk` as the provider.
- `-H`: pass this flag is you want to only measure the timing iteration of a
benchmark.
By default, the tracing tools will measure the entire execution.
- `-p`: print the entire tracing script before execution.
This is mainly for debugging use.
- `-f`: change the bpftrace output format.
By default, it uses human-readable plain text output (`text`).
You can set this to `json` for easy parsing.

Please run the tracing tools **before** running the workload.
If you use `-H`, the tracing tools will automatically end with `harness_end` is
called.
Otherwise, you will need to terminate the tools manually with `Ctrl-C`.
These tools also have a timeout of 1200 seconds so not to stall unattended
benchmark execution.

## Tracing tools
### Measuring the time spend in allocation slow path (`alloc_slow`)
This tool measures the distribution of the allocation slow path time.
The time unit is 400ns, so that we use the histogram bins with higher
fidelity better.

Sample output:
```
@alloc_slow_hist:
[4, 8)               304 |@                                                   |
[8, 16)            12603 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@|
[16, 32)            8040 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@                   |
[32, 64)             941 |@@@                                                 |
[64, 128)            171 |                                                    |
[128, 256)            13 |                                                    |
[256, 512)             2 |                                                    |
[512, 1K)              0 |                                                    |
[1K, 2K)               0 |                                                    |
[2K, 4K)               0 |                                                    |
[4K, 8K)               0 |                                                    |
[8K, 16K)              0 |                                                    |
[16K, 32K)            14 |                                                    |
[32K, 64K)            37 |                                                    |
[64K, 128K)           19 |                                                    |
[128K, 256K)           1 |                                                    |
```

In the above output, we can see that most allocation slow paths finish between
3.2us and 6.4us.
However, there is a long tail, presumably due to GC pauses.

### Measuring the time spend in different GC stages (`gc_stages`)
This tool measures the time spent in different stages of GC: before `Closure`,
during `Closure`, and after `Closure`.
The time unit is ns.

Sample output:
```
@closure_time: 1405302743
@post_closure_time: 81432919
@pre_closure_time: 103886118
```

In the above output, overall, the execution spends 1.4s in the main transitive
closure, 103ms before that, and 81ms after that (a total of around 1.5s).

### Measuring the time spend in lock contended state for Rust `Mutex` (`lock_contend`)
This tools measures the time spent in the lock contended state for Rust `Mutex`s.
The Rust standard library implements `Mutex` using the fast-slow-path paradigm.
Most lock operations take place in inlined fast paths, when there's no contention.
However, when there's contention,
`std::sys::unix::locks::futex_mutex::Mutex::lock_contended` is called.

```rust
#[inline]
pub fn lock(&self) {
    if self.futex.compare_exchange(0, 1, Acquire, Relaxed).is_err() {
        self.lock_contended();
    }
}

#[cold]
fn lock_contended(&self) {
    <snip>
}
```


MMTk uses Rust `Mutex`, e.g., in allocation slow paths for synchronization,
and this tool can be useful to measure the contention in these parts of code.

The time unit is 256ns.

Sample output:
```
@lock_dist[140637228007056]: 
[1]                  447 |@@@@                                                |
[2, 4)              3836 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@             |
[4, 8)              3505 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@                |
[8, 16)             1354 |@@@@@@@@@@@@@@                                      |
[16, 32)             832 |@@@@@@@@                                            |
[32, 64)            1077 |@@@@@@@@@@@                                         |
[64, 128)           2991 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@                     |
[128, 256)          4846 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@  |
[256, 512)          5013 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@|
[512, 1K)           1203 |@@@@@@@@@@@@                                        |
[1K, 2K)              34 |                                                    |
[2K, 4K)              15 |                                                    |
```

In the above output, we can see that the lock instance (140637228007056, or 0x7fe8a8047e90)
roughly has a bimodal distribution in terms of the time spent in lock contended
code path.
The first peak is around 512ns\~1024ns, and the second peak is around 66us\~131us.

If you can't tell which lock instance is for which lock in MMTk, you can trace
the allocation of the Mutex and record the stack trace (note that you might want
to compile MMTk with `force-frame-pointers` to obtain better stack traces).

### Measuring the distribution of `process_edges` packet sizes (`packet_size`)
Most of the GC time is spend in the transitive closure for tracing-based GCs,
and MMTk performs transitive closure via work packets that calls the `process_edges` method.
This tool measures the distribution of the sizes of these work packets, and also
count root edges separately.

Sample output:
```
@process_edges_packet_size:
[1]                  238 |@@@@@                                               |
[2, 4)               806 |@@@@@@@@@@@@@@@@@                                   |
[4, 8)              1453 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@                     |
[8, 16)             1105 |@@@@@@@@@@@@@@@@@@@@@@@                             |
[16, 32)            2410 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@|
[32, 64)            1317 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@                        |
[64, 128)           1252 |@@@@@@@@@@@@@@@@@@@@@@@@@@@                         |
[128, 256)          1131 |@@@@@@@@@@@@@@@@@@@@@@@@                            |
[256, 512)          2017 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@         |
[512, 1K)           1270 |@@@@@@@@@@@@@@@@@@@@@@@@@@@                         |
[1K, 2K)            1028 |@@@@@@@@@@@@@@@@@@@@@@                              |
[2K, 4K)             874 |@@@@@@@@@@@@@@@@@@                                  |
[4K, 8K)            1024 |@@@@@@@@@@@@@@@@@@@@@@                              |
[8K, 16K)             58 |@                                                   |
[16K, 32K)             5 |                                                    |

@process_edges_root_packet_size:
[1]                   71 |@@@@@@@                                             |
[2, 4)                 4 |                                                    |
[4, 8)               276 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@                        |
[8, 16)              495 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@|
[16, 32)             477 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@  |
[32, 64)             344 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@                |
[64, 128)            242 |@@@@@@@@@@@@@@@@@@@@@@@@@                           |
[128, 256)           109 |@@@@@@@@@@@                                         |
[256, 512)            31 |@@@                                                 |
[512, 1K)             33 |@@@                                                 |
[1K, 2K)              75 |@@@@@@@                                             |
[2K, 4K)              75 |@@@@@@@                                             |
[4K, 8K)             336 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@                 |
[8K, 16K)             56 |@@@@@                                               |
[16K, 32K)             3 |                                                    |
```

In the above output, we can see that overall, the sizes of the `process_edges`
has a unimodal distribution with a peak around 16\~32 edges per packet.
However, if we focus on root edges, the distribution is roughly bimodal, with a
first peak around 8\~16 and a second peak around 4096\~8192.

