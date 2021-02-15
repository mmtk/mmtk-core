pub mod gc_count;
pub mod obj_num;
pub mod obj_size;

/**
 * This trait exposes hooks for developers to implement their own analysis routines.
 * Note that the trait itself takes generic parameters that are used as the argument
 * types for its hooks. This allows for a general framework wherein, if a developer
 * chooses, multiple arguments can be passed to the analysis routine by packing them
 * in a struct (for an example see the concrete implementation of
 * PerSizeClassObjectCounter).
 *
 * Most traits would want to hook into the `Stats` and counters provided by the MMTk
 * framework that are exposed to the Harness.
 *
 * Generic parameter A -> Allocation hook
 * Generic parameter C -> Collection hook
 */
pub trait RtAnalysis<A, C> {
    fn alloc_hook(&mut self, _args: A) {}
    fn gc_hook(&mut self, _args: C) {}
}
