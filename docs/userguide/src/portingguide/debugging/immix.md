# Debugging Copying in Immix Plans

Immix uses opportunitic copying, which means it does not always copy objects.
So a bug related with copying may be non-deterministic with Immix plans.

One way to make copying more deterministic is to use the following
[options](https://docs.mmtk.io/api/mmtk/util/options/struct.Options.html) to
change the copying behavior of Immix.

| Option                          | Default Value   | Note                                                                        |
|---------------------------------|-----------------|-----------------------------------------------------------------------------|
| `immix_always_defrag`           | `false`         | Immix only does defrag GC when necessary. Set to `true` to make every GC a defrag GC |
| `immix_defrag_every_block`      | `false`         | Immix only defrags the most heavily fragmented blocks. Set to `true` to make Immix defrag every block with equal chances  |
| `immix_defrag_headroom_percent` | `2`             | Immix uses 2% of the heap for defraging. We can reserve more headroom to copy more objects. 50% makes Immix behave like SemiSpace. |

A common way to maximumally expose Immix copying bugs is to run with the following values:
```rust
// Set options with MMTkBuilder
builder.options.immix_always_defrag.set(true);
builder.options.immix_defrag_every_block.set(true);
builder.options.immix_defrag_headroom_percent.set(50);
```

These options can also be used along with stress GC options:
```rust
// Do a stress GC for every 10MB allocation
builder.options.stress_factor.set(10485760);
```

Options can also be set using environment variables.
```console
export MMTK_IMMIX_ALWAYS_DEFRAG=true
export MMTK_IMMIX_DEFRAG_EVERY_BLOCK=true
export MMTK_IMMIX_DEFRAG_HEADROOM_PERCENT=50
export MMTK_STRESS_FACTOR=10485760
```

