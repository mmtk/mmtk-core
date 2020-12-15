# Preliminaries
## Set up MMTK-core and binding
1.	Recommend use of openjdk because it has the most implemented features, some things in tute will specifically refer to openjdk but the tute can be done with any binding. 
a.	Build debug option: Need to define a build to use
2.	Make sure openjdk setup works (use tests from binding readme). Explain how to do multiple builds? (Maybe raise as issue for binding readme instead)
## Create mygc
1.	Copy mmtk-openjdk/repos/mmtk-core/src/plan/nogc
2.	Refactor name in all files in plan to mygc
3.	Maybe remove extra features (lock-free etc) because they are not needed for the tutorial?
4.	Add mygc to cargo, core plan cargo, plan/mod
5.	Bring attention to important aspects within plan?
* Where is the allocator? (mutator.rs – note lack of r/w barrier)
*	What happens if garbage has to be collected?
*	Talk about aspects of constructors?
# Turn NoGC into Semispace
## Allocation: Add copyspaces
Add the two copyspaces and change the alloc/mut to work with these spaces
1. global.rs: add imports (CommonPlan, AtomicBool)
*	pub struct MyGC: Remove old. Add copyspaces. Add ‘hi’ to/from indicator. Replace base plan with common plan.
*	impl Plan for MyGC: new: init things. gc_init: init things.
2.	mutator.rs
*	change value maps in lazy_static - going to need different space types for SemiSpace. 
*	create_mygc_mutator: Change space_mapping. tospace gets an immortal space, fromspace gets a large-object space (los). Only from is going to have a collection in it. To and from are swapped each collection, and are of equal size. This means that there’s no chance for tospace to run out of memory, but it isn’t the most efficient system.
3. add mut prep/release functions
4.	Test allocation is working
*	How?
## Collector: Implement garbage collection
1.	Implement work packet. Make new file gc_works. This file implements CopyContext and ProcessEdges. The former provides context for when the gc needs to collect, and ProcessEdges ?
## Adding another copyspace
Less guided exercise: Add “young” copyspace which all new objects go to before moving to the fromspace. 

Further reading: 
- GC handbook ([O’Reilly access](https://learning.oreilly.com/library/view/the-garbage-collection/9781315388007/?ar))
- Videos: [MPLR 2020 Keynote](https://www.youtube.com/watch?v=3L6XEVaYAmU) [Deconstructing the Garbage-First Collector](https://www.youtube.com/watch?v=MAk6RdApGLs)
-	Original MMTK papers
