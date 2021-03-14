# Next Steps

Your choice of the next GC plan to implement depends on your situation.
If you’re developing a new VM from scratch, or if you are intimately familiar with the internals of your target VM, then implementing a SemiSpace collector is probably the best course of action.
Although the GC itself is rather simplistic, it stresses many of the key components of the MMTk <-> VM binding that will be required for later (and more powerful) GCs.
In particular, since it always moves objects, it is an excellent stress test.

An alternative route is to implement MarkSweep.
This may be necessary in scenarios where the target VM doesn’t support object movement, or would require significant refactoring to do so.
This can then serve as a stepping stone for future, moving GCs such as SemiSpace. 

We hope to have an Immix implementation available soon, which provides a nice middle ground between moving and non-moving (since it copies opportunistically, and can cope with a strictly non-moving requirement if needs be).
