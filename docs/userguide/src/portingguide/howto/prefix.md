# How to Undertake a Port

We recommend a highly incremental approach to implementing a port.   The broad idea is:
 - Start with the NoGC plan and gradually move to more advanced collectors
 - Focus on simplicity and correctness.
 - Optimize the port later.

In MMTk’s language, a plan is essentially a configuration which specifies a GC algorithm.
Plans can be selected at run time.
Not all plans will be suitable for all runtimes.
For example, a runtime that for some reason cannot support object movement won’t be able to use plans that use copying garbage collection.
