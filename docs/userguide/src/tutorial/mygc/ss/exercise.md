# Exercise: Adding another copyspace

Now that you have a working semispace collector, you should be familiar 
enough with the code to start writing some yourself. The intention of this 
exercise is to reinforce the information from the semispace section, rather 
than to create a useful new collector.

1. Create a copy of your semispace collector, called `triplespace`. 
2. Add a new copyspace to the collector, called the `youngspace`, with the 
following traits:
    * New objects are allocated to the youngspace (rather than the fromspace).
    * During a collection, live objects in the youngspace are moved to the 
    tospace.
    * Garbage is still collected at the same time for all spaces.

Triplespace is a sort of generational garbage collector. These collectors 
separate out old objects and new objects into separate spaces. Newly 
allocated objects should be scanned far more often than old objects, which 
minimises the time spent repeatedly re-scanning long-lived objects. 

Of course, this means that the Triplespace is incredibly inefficient for a 
generational collector, because the older objects are still being scanned 
every collection. It wouldn't be very useful in a real-life scenario. The 
next thing to do is to make this collector into a more efficient proper 
generational collector.

When you are finished, try running the benchmarks and seeing how the 
performance of this collector compares to MyGC. Great work!