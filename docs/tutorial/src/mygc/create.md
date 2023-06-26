# Create MyGC

NoGC is a GC plan that only allocates memory, and does not have a collector. 
We're going to use it as a base for building a new garbage collector.

Recall that this tutorial will take you through the steps of building a 
collector from basic principles. To do that, you'll create your own plan 
called `MyGC` which you'll gradually refine and improve upon through the 
course of this tutorial. At the beginning MyGC will resemble the very 
simple NoGC plan.

1. Each plan is stored in `mmtk-openjdk/repos/mmtk-core/src/plan`. Navigate 
there and create a copy of the folder `nogc`. Rename it to `mygc`.
3. In *each file* within `mygc`, rename any reference to `nogc` to `mygc`. 
You will also have to separately rename any reference to `NoGC` to `MyGC`.
   * For example, in Visual Studio Code, you can (making sure case sensitivity 
   is selected in the search function) select one instance of `nogc` and either 
   right click and select "Change all instances" or use the CTRL-F2 shortcut, 
   and then type `mygc`, and repeat for `NoGC`.
4. In order to use MyGC, you will need to make some changes to the following 
files. 
    1. `mmtk-core/src/plan/mod.rs`, add:
        ```rust
        pub mod mygc;
        ```
        This adds `mygc` as a module.
    1. `mmtk-core/src/util/options.rs`, add `MyGC` to the enum `PlanSelector`. 
    This allows MMTk to accept `MyGC` as a command line option for `plan`, 
    or an environment variable for `MMTK_PLAN`:
        ```rust
        #[derive(Copy, Clone, EnumFromStr, Debug)]
        pub enum PlanSelector {
            NoGC,
            SemiSpace,
            GenCopy,
            GenImmix,
            MarkSweep,
            PageProtect,
            Immix,
            // Add this!
            MyGC,
        }
        ```
    1. `mmtk-core/src/plan/global.rs`, add new expressions to 
    `create_mutator()` and `create_plan()` for `MyGC`, following the pattern of 
    the existing plans. These define the location of the mutator and plan's 
    constructors. 
        ```rust
        // NOTE: Sections of this code snippet not relevant to this step of the 
        // tutorial (marked by "// ...") have been omitted.
        pub fn create_mutator<VM: VMBinding>(
            tls: VMMutatorThread,
            mmtk: &'static MMTK<VM>,
        ) -> Box<Mutator<VM>> {
            Box::new(match mmtk.options.plan {
                PlanSelector::NoGC => crate::plan::nogc::mutator::create_nogc_mutator(tls, &*mmtk.plan),
                PlanSelector::SemiSpace => {
                    crate::plan::semispace::mutator::create_ss_mutator(tls, &*mmtk.plan)
                }

                // ...

                // Create MyGC mutator based on selector
                PlanSelector::MyGC => crate::plan::mygc::mutator::create_mygc_mutator(tls, &*mmtk.plan),    })
        }

        pub fn create_plan<VM: VMBinding>(
            plan: PlanSelector,
            vm_map: &'static VMMap,
            mmapper: &'static Mmapper,
            options: Arc<UnsafeOptionsWrapper>,
        ) -> Box<dyn Plan<VM = VM>> {
            match plan {
                PlanSelector::NoGC => Box::new(crate::plan::nogc::NoGC::new(args)),
                PlanSelector::SemiSpace => Box::new(crate::plan::semispace::SemiSpace::new(args)),

                // ...

                // Create MyGC plan based on selector
                PlanSelector::MyGC => Box::new(crate::plan::mygc::MyGC::new(args))
            }
        }       
        ```
    
Note that all of the above changes almost exactly copy the NoGC entries in 
each of these files. However, NoGC has some variants, such as a lock-free 
variant. For simplicity, those are not needed for this tutorial. Remove 
references to them in the MyGC plan now. 

1. Within `mygc/global.rs`, find any use of `#[cfg(feature = "mygc_lock_free")]` 
and delete both it *and the line below it*.
2. Then, delete any use of the above line's negation, 
`#[cfg(not(feature = "mygc_lock_free"))]`, this time without changing the 
line below it.

After you rebuild OpenJDK (and `mmtk-core`), you can run MyGC with your new 
build (`MMTK_PLAN=MyGC`). Try testing it with the each of the three benchmarks. 
It should work identically to NoGC.

If you've got to this point, then congratulations! You have created your first 
working MMTk collector!


At this point, you should familiarise yourself with the MyGC plan if you 
haven't already. Try answering the following questions by looking at the code 
and [Further Reading](../further_reading.md): 

   * Where is the allocator defined?
   * How many memory spaces are there?
   * What kind of memory space policy is used?
   * What happens if garbage has to be collected?   
