# Glossary

*allocator*: Code that allocates new objects into memory.

*collector*: Finds and frees memory occupied by 'dead' objects. 

*dead*: An object that is not live.

*GC work (unit), GC packet*: A schedulable unit of collection work. 

*GC worker*: A worker thread that performs garbage collection operations 
(as required by GC work units).

*live*: An object that is reachable, and thus can still be accessed by other 
objects, is live/alive.

*mutator*: Something that 'mutates', or changes, the objects stored in memory. 
This is the term that is traditionally used in the garbage collection literature 
to describe the running program (because it 'mutates' the object graph).

*plan*: A garbage collection algorithm expressed as a configuration of policies. 
See also [Plans and policies](#plans-and-policies) below.

*policy*: A specific garbage collection algorithm, such as marksweep, copying, 
immix, etc. Plans are made up of an arrangement of one or more policies. 
See also [Plans and policies](#plans-and-policies) below.

*scheduler*: Dynamically dispatches units of GC work to workers.

*zeroing*, *zero initialization*: Initializing and resetting unused memory 
bits to have a value of 0. Required by most memory-safe programming languages.

See also: [Further Reading](../further_reading.md)


## Plans and Policies

In MMTk, collectors are instantiated as plans, which can be thought of as 
configurations of collector policies. In practice, most production 
collectors and almost all collectors in MMTk are comprised of multiple 
algorithms/policies. For example the gencopy plan describes a configuration 
that combines a copying nursery with a semispace mature space. In MMTk we 
think of these as three spaces, each of which happen to use the copyspace 
policy, and which have a relationship which is defined by the gencopy plan. 
Under the hood, gencopy builds upon a common plan which may also contain other 
policies including a space for code, a read-only space, etc.

Thus, someone wishing to construct a new collector based entirely on existing 
policies may be able to do so in MMTk by simply writing a new plan, which is 
what this tutorial covers.

On the other hand, someone wishing to introduce an entirely new garbage 
collection policy (such as Immix, for example), would need to first create 
a policy which specifies that algorithm, before creating a plan which defines 
how the GC algorithm fits together and utilizes that policy.