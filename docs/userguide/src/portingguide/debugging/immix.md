# Debugging Copying in Immix Plans

Immix uses opportunitic copying, which means it does not always copy objects.
So a bug related with copying may be non-deterministic with Immix plans.

One way to make copying more deterministic is to use the following options to
change the copying behavior of Immix.

| Option                          | Default Value   | Note                                                                        |
|---------------------------------|-----------------|-----------------------------------------------------------------------------|
| `immix_stress_defrag`           | `false`         | Immix only does defrag GC when necessary. Set to `true` to make every GC a defrag GC |
| `immix_defrag_every_block`      | `false`         | Immix only defrags the most heavily fragmented blocks. Set to `true` to make Immix defrag every block with equal chances  |
| `immix_defrag_headroom_percent` | `2`             | Immix uses 2% of the heap for defraging. We can reserve more headroom to copy all objects. 50% makes Immix behave like SemiSpace. |

A common way to maximumally expose Immix copying bugs is to run with the following values:
* `immix_stress_defrag` = `true`
* `immix_defrag_every_block` = `true`
* `immix_defrag_headroom_percent` = `50`

These options can also be used along with stress GC options:
* `stress_factor` = `10485760` (Do a GC for every 10MB)
