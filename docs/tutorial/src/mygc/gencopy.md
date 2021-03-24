# Building a generational copying collector

> Note: This part is work in progress.

## What is a generational collector?

The *weak generational hypothesis* states that most of the objects allocated
to a heap after one collection will die before the next collection.
Therefore, it is worth separating out 'young' and 'old' objects and only
scanning each as needed, to minimise the number of times old live objects are
scanned. New objects are allocated to a 'nursery', and after one collection
they move to the 'mature' space. In `triplespace`, `youngspace` is a
proto-nursery, and the `tospace` and `fromspace` are the mature spaces.

This collector fixes one of the major problems with semispace - namely, that
any long-lived objects are repeatedly copied back and forth. By separating
these objects into a separate 'mature' space, the number of full heap
collections needed is greatly reduced.

This section is currently incomplete. Instructions for building a
generational copying (gencopy) collector will be added in future.
