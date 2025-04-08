searchState.loadedDescShard("mmtk", 1, "The type of finalizable objects. This type is used when …\nVM-specific methods for reference processing, including …\nWeak and soft references always clear the referent before …\nFor weak reference types, if the referent is cleared …\nLoad the object reference.\nGet the referent from a weak reference object.\nKeep the heap references in the finalizable object alive. …\nStore the object reference.\nSet the referent in a weak reference object.\nCallback trait of scanning functions that directly trace …\nAn <code>ObjectTracerContext</code> gives a GC worker temporary access …\nRoot-scanning methods use this trait to create work …\nVM-specific methods for scanning roots/objects.\nCallback trait of scanning functions that report slots.\nThe concrete <code>ObjectTracer</code> type.\nWhen set to <code>true</code>, all plans will guarantee that during …\nWhen set to <code>true</code>, all plans will guarantee that during …\nCreate work packets to handle non-transitively pinning …\nCreate work packets to handle non-pinned roots.  The roots …\nCreate work packets to handle transitively pinning (TP) …\nForward weak references.\nForward weak references.\nMMTk calls this method at the first time during a …\nPrepare for another round of root scanning in the same GC. …\nProcess weak references.\nProcess weak references.\nDelegated scanning of a object, visiting each reference …\nDelegated scanning of a object, visiting each reference …\nDelegated scanning of a object, visiting each reference …\nScan one mutator for stack roots.\nScan VM-specific roots. The creation of all root scan …\nReturn true if the given object supports slot enqueuing.\nReturn true if the given object supports slot enqueuing.\nReturn whether the VM supports return barriers. This is …\nCall this function to trace through an object graph edge …\nCall this function for each slot.\nCreate a temporary <code>ObjectTracer</code> and provide access in the …\nIterate slots within <code>Range&lt;Address&gt;</code>.\nA abstract memory slice represents a piece of <strong>heap</strong> memory …\nA simple slot implementation that represents a word-sized …\nA <code>Slot</code> represents a slot in an object (a.k.a. a field), on …\nThe associate type to define how to iterate slots in a …\nThe associate type to define how to access slots from a …\nMemory slice type with empty implementations. For VMs that …\nSlot iterator for <code>UnimplementedMemorySlice</code>.\nGet the address of the slot.\nSize of the memory slice\nMemory copy support\nReturns the argument unchanged.\nReturns the argument unchanged.\nReturns the argument unchanged.\nReturns the argument unchanged.\nCreate a simple slot from an address.\nCalls <code>U::from(self)</code>.\nCalls <code>U::from(self)</code>.\nCalls <code>U::from(self)</code>.\nCalls <code>U::from(self)</code>.\nIterate object slots within the slice. If there are …\nLoad object reference from the slot.\nThe object which this slice belongs to. If we know the …\nPrefetch the slot so that a subsequent <code>load</code> will be faster.\nPrefetch the slot so that a subsequent <code>store</code> will be …\nStart address of the memory slice\nStore the object reference <code>object</code> into the slot.")