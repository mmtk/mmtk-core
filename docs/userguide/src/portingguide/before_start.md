# Things to Consider Before Starting a Port

In principle, a port to MMTk is not particularly difficult.
MMTk can present itself as a standard library and the core of the API is relatively simple.

However, porting a runtime to a different GC (any GC) can be difficult and time consuming.
Key questions include: 
 - How well encapsulated is the runtime's existing collector? 
 - Does the runtime make tacit assumptions about the underlying collector's implementation?
 - How many places in the runtime codebase reference some part of the GC?
 - If the runtime has a JIT, how good is the interface between the JIT and the GC (for write barriers and allocations, for example)?
 - Does the runtime support precise stack scanning? 
 - etc.

Thinking through these questions should give you a sense for how big a task a GC port will be.