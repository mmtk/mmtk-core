# What *is* MMTk?

The Memory Management Toolkit (MMTk) is a framework for designing and 
implementing memory managers. It has a runtime-neutral core (mmtk-core) 
written in Rust, and bindings that allow it to work with OpenJDK, V8, 
and JikesRVM, with more bindings currently in development. 
MMTk was originally written in Java as part of the JikesRVM Java runtime.
The current version is similar in its purpose, but was made to be 
very flexible with runtime and able to be ported to many different VMs.

The principal idea of MMTk is that it can be used as a 
toolkit, allowing new GC algorithms to be rapidly developed using 
common components. It also allows different GC algorithms to be 
compared on an apples-to-apples basis, since they share common mechanisms.