# MyGC

In this section, we will build a new garbage collector from scratch, called
`MyGC`. We will start by creating MyGC as a copy of the simple NoGC plan,
which only allocates memory and never collects. We will then extend it step
by step into a semispace collector, and finally into a generational copying
collector.
