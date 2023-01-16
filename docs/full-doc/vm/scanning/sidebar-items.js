initSidebarItems({"trait":[["EdgeVisitor","Callback trait of scanning functions that report edges."],["ObjectTracer","Callback trait of scanning functions that directly trace through edges."],["ObjectTracerContext","An `ObjectTracerContext` gives a GC worker temporary access to an `ObjectTracer`, allowing the GC worker to trace objects.  This trait is intended to abstract out the implementation details of tracing objects, enqueuing objects, and creating work packets that expand the transitive closure, allowing the VM binding to focus on VM-specific parts."],["RootsWorkFactory","Root-scanning methods use this trait to create work packets for processing roots."],["Scanning","VM-specific methods for scanning roots/objects."]]});