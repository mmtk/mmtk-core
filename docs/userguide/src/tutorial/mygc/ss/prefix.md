# Building a semispace collector

In a semispace collector, the heap is divided into two equally-sized spaces, 
called 'semispaces'. One of these is defined as a 'fromspace', and the other 
a 'tospace'. The allocator allocates to the tospace until it is full. 

When the tospace is full, a stop-the-world GC is triggered. The mutator is 
paused, and the definitions of the spaces are flipped (the 'tospace' becomes 
a 'fromspace', and vice versa). Then, the collector scans each object in what 
is now the fromspace. If a live object is found, a copy of it is made in the 
tospace. That is to say, live objects are copied *from* the fromspace *to* 
the tospace. After every object is scanned, the fromspace is cleared. The GC 
finishes, and the mutator is resumed.
