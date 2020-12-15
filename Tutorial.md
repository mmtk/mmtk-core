Recommended resources:
•	GC handbook (O’Reilly access)
•	Steve’s yt channel for mmtk talks
•	Original MMTK papers

1.	Recommend use of openjdk because it has the most implemented features, some things in tute will specifically refer to openjdk but the tute can be done with any binding. 
a.	Build debug option: fast-debug for testing speed? Or slow-debug for faster build time? First build will take a long time regardless but further builds do not take as long. Only need to rebuild ‘build’ folder.
2.	Make sure openjdk setup works (use tests from binding readme). Explain how to do multiple builds? (Maybe raise as issue for binding readme instead)
3.	Copy mmtk-openjdk/repos/mmtk-core/src/plan/nogc
a.	Refactor name in all files in plan to mygc
b.	Remove all these extra features cause we don’t need em for the explanation?
4.	Fix up cargo, core plan cargo, plan/mod
5.	Bring attention to important aspects within plan
a.	Where is the allocator? (mutator.rs – note lack of r/w barrier)
b.	What happens if garbage has to be collected?
c.	Talk about aspects of constructors (sidebar?)
SEMISPACE
6.	Start turning nogc into semispace by adding the two copyspaces and changing the alloc/mut to work with these spaces
a.	global
i.	add imports (CommonPlan, AtomicBool)
ii.	pub struct MyGC: Remove old. Add copyspaces. Add ‘hi’ to/from indicator. Replace base plan with common plan.
iii.	impl Plan for MyGC:
1.	new: init things
2.	gc_init: init things
b.	mutator
i.	change value maps in lazy_static
1.	Going to need different space types for SemiSpace. 
ii.	create_mygc_mutator
1.	Change space_mapping
a.	tospace gets an immortal space, fromspace gets a large-object space (los). Only from is going to have a collection in it. To and from are swapped each collection, and are of equal size. This means that there’s no chance for tospace to run out of memory, but it isn’t the most efficient system.
iii.	add mut prep/release functions
7.	Test allocation is working
a.	How?
8.	Add collector
a.	Implement work packet. Make new file gc_works. This file implements CopyContext and ProcessEdges. The former provides context for when the gc needs to collect, and ProcessEdges ?
9.	(end of semispace bit) Add sanity gc?
10.	Less guided exercise: Add “young” copyspace which all new objects go to before moving to the fromspace. 

