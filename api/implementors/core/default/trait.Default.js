(function() {var implementors = {
"mmtk":[["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/heap/chunk_map/struct.ChunkMap.html\" title=\"struct mmtk::util::heap::chunk_map::ChunkMap\">ChunkMap</a>"],["impl&lt;F: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> + <a class=\"trait\" href=\"mmtk/vm/reference_glue/trait.Finalizable.html\" title=\"trait mmtk::vm::reference_glue::Finalizable\">Finalizable</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/finalizable_processor/struct.FinalizableProcessor.html\" title=\"struct mmtk::util::finalizable_processor::FinalizableProcessor\">FinalizableProcessor</a>&lt;F&gt;"],["impl&lt;T&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/plan/tracing/struct.VectorQueue.html\" title=\"struct mmtk::plan::tracing::VectorQueue\">VectorQueue</a>&lt;T&gt;"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/plan/mutator_context/struct.ReservedAllocators.html\" title=\"struct mmtk::plan::mutator_context::ReservedAllocators\">ReservedAllocators</a>"],["impl&lt;VM: <a class=\"trait\" href=\"mmtk/vm/trait.VMBinding.html\" title=\"trait mmtk::vm::VMBinding\">VMBinding</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/plan/gc_requester/struct.GCRequester.html\" title=\"struct mmtk::plan::gc_requester::GCRequester\">GCRequester</a>&lt;VM&gt;"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/scheduler/gc_work/struct.ReleaseCollector.html\" title=\"struct mmtk::scheduler::gc_work::ReleaseCollector\">ReleaseCollector</a>"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/analysis/obj_size/struct.PerSizeClassObjectCounter.html\" title=\"struct mmtk::util::analysis::obj_size::PerSizeClassObjectCounter\">PerSizeClassObjectCounter</a>"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/opaque_pointer/struct.OpaquePointer.html\" title=\"struct mmtk::util::opaque_pointer::OpaquePointer\">OpaquePointer</a>"],["impl&lt;VM: <a class=\"trait\" href=\"mmtk/vm/trait.VMBinding.html\" title=\"trait mmtk::vm::VMBinding\">VMBinding</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/copy/struct.CopyConfig.html\" title=\"struct mmtk::util::copy::CopyConfig\">CopyConfig</a>&lt;VM&gt;"],["impl&lt;E: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> + <a class=\"trait\" href=\"mmtk/scheduler/gc_work/trait.ProcessEdgesWork.html\" title=\"trait mmtk::scheduler::gc_work::ProcessEdgesWork\">ProcessEdgesWork</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/reference_processor/struct.WeakRefProcessing.html\" title=\"struct mmtk::util::reference_processor::WeakRefProcessing\">WeakRefProcessing</a>&lt;E&gt;"],["impl&lt;E: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> + <a class=\"trait\" href=\"mmtk/scheduler/gc_work/trait.ProcessEdgesWork.html\" title=\"trait mmtk::scheduler::gc_work::ProcessEdgesWork\">ProcessEdgesWork</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/finalizable_processor/struct.ForwardFinalization.html\" title=\"struct mmtk::util::finalizable_processor::ForwardFinalization\">ForwardFinalization</a>&lt;E&gt;"],["impl&lt;VM: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> + <a class=\"trait\" href=\"mmtk/vm/trait.VMBinding.html\" title=\"trait mmtk::vm::VMBinding\">VMBinding</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/analysis/struct.AnalysisManager.html\" title=\"struct mmtk::util::analysis::AnalysisManager\">AnalysisManager</a>&lt;VM&gt;"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/analysis/struct.GcHookWork.html\" title=\"struct mmtk::util::analysis::GcHookWork\">GcHookWork</a>"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/policy/immix/defrag/struct.Defrag.html\" title=\"struct mmtk::policy::immix::defrag::Defrag\">Defrag</a>"],["impl&lt;VM: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> + <a class=\"trait\" href=\"mmtk/vm/trait.VMBinding.html\" title=\"trait mmtk::vm::VMBinding\">VMBinding</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/reference_processor/struct.RefEnqueue.html\" title=\"struct mmtk::util::reference_processor::RefEnqueue\">RefEnqueue</a>&lt;VM&gt;"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"enum\" href=\"mmtk/util/alloc/allocators/enum.AllocatorInfo.html\" title=\"enum mmtk::util::alloc::allocators::AllocatorInfo\">AllocatorInfo</a>"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/mmtk/struct.MMTKBuilder.html\" title=\"struct mmtk::mmtk::MMTKBuilder\">MMTKBuilder</a>"],["impl&lt;ES: <a class=\"trait\" href=\"mmtk/vm/edge_shape/trait.Edge.html\" title=\"trait mmtk::vm::edge_shape::Edge\">Edge</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/sanity/sanity_checker/struct.SanityChecker.html\" title=\"struct mmtk::util::sanity::sanity_checker::SanityChecker\">SanityChecker</a>&lt;ES&gt;"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/metadata/side_metadata/sanity/struct.SideMetadataSanity.html\" title=\"struct mmtk::util::metadata::side_metadata::sanity::SideMetadataSanity\">SideMetadataSanity</a>"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/options/struct.Options.html\" title=\"struct mmtk::util::options::Options\">Options</a>"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/heap/gc_trigger/struct.MemBalancerStats.html\" title=\"struct mmtk::util::heap::gc_trigger::MemBalancerStats\">MemBalancerStats</a>"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/heap/layout/byte_map_mmapper/struct.ByteMapMmapper.html\" title=\"struct mmtk::util::heap::layout::byte_map_mmapper::ByteMapMmapper\">ByteMapMmapper</a>"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/heap/layout/vm_layout/struct.VMLayout.html\" title=\"struct mmtk::util::heap::layout::vm_layout::VMLayout\">VMLayout</a>"],["impl&lt;Edges: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> + <a class=\"trait\" href=\"mmtk/scheduler/gc_work/trait.ProcessEdgesWork.html\" title=\"trait mmtk::scheduler::gc_work::ProcessEdgesWork\">ProcessEdgesWork</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/scheduler/gc_work/struct.ScanVMSpecificRoots.html\" title=\"struct mmtk::scheduler::gc_work::ScanVMSpecificRoots\">ScanVMSpecificRoots</a>&lt;Edges&gt;"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/scheduler/work_counter/struct.WorkCounterBase.html\" title=\"struct mmtk::scheduler::work_counter::WorkCounterBase\">WorkCounterBase</a>"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/scheduler/stat/struct.SchedulerStat.html\" title=\"struct mmtk::scheduler::stat::SchedulerStat\">SchedulerStat</a>"],["impl&lt;VM: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> + <a class=\"trait\" href=\"mmtk/vm/trait.VMBinding.html\" title=\"trait mmtk::vm::VMBinding\">VMBinding</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/scheduler/gc_work/struct.VMPostForwarding.html\" title=\"struct mmtk::scheduler::gc_work::VMPostForwarding\">VMPostForwarding</a>&lt;VM&gt;"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/scheduler/gc_work/struct.PrepareCollector.html\" title=\"struct mmtk::scheduler::gc_work::PrepareCollector\">PrepareCollector</a>"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/heap/accounting/struct.PageAccounting.html\" title=\"struct mmtk::util::heap::accounting::PageAccounting\">PageAccounting</a>"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/heap/layout/map32/struct.Map32.html\" title=\"struct mmtk::util::heap::layout::map32::Map32\">Map32</a>"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"enum\" href=\"mmtk/util/copy/enum.CopySelector.html\" title=\"enum mmtk::util::copy::CopySelector\">CopySelector</a>"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/reference_processor/struct.ReferenceProcessors.html\" title=\"struct mmtk::util::reference_processor::ReferenceProcessors\">ReferenceProcessors</a>"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/heap/layout/fragmented_mapper/struct.FragmentedMapper.html\" title=\"struct mmtk::util::heap::layout::fragmented_mapper::FragmentedMapper\">FragmentedMapper</a>"],["impl&lt;E: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> + <a class=\"trait\" href=\"mmtk/scheduler/gc_work/trait.ProcessEdgesWork.html\" title=\"trait mmtk::scheduler::gc_work::ProcessEdgesWork\">ProcessEdgesWork</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/finalizable_processor/struct.Finalization.html\" title=\"struct mmtk::util::finalizable_processor::Finalization\">Finalization</a>&lt;E&gt;"],["impl&lt;E: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> + <a class=\"trait\" href=\"mmtk/scheduler/gc_work/trait.ProcessEdgesWork.html\" title=\"trait mmtk::scheduler::gc_work::ProcessEdgesWork\">ProcessEdgesWork</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/reference_processor/struct.RefForwarding.html\" title=\"struct mmtk::util::reference_processor::RefForwarding\">RefForwarding</a>&lt;E&gt;"],["impl&lt;ScanEdges: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> + <a class=\"trait\" href=\"mmtk/scheduler/gc_work/trait.ProcessEdgesWork.html\" title=\"trait mmtk::scheduler::gc_work::ProcessEdgesWork\">ProcessEdgesWork</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/scheduler/gc_work/struct.StopMutators.html\" title=\"struct mmtk::scheduler::gc_work::StopMutators\">StopMutators</a>&lt;ScanEdges&gt;"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/heap/heap_meta/struct.HeapMeta.html\" title=\"struct mmtk::util::heap::heap_meta::HeapMeta\">HeapMeta</a>"],["impl&lt;E: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> + <a class=\"trait\" href=\"mmtk/scheduler/gc_work/trait.ProcessEdgesWork.html\" title=\"trait mmtk::scheduler::gc_work::ProcessEdgesWork\">ProcessEdgesWork</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/reference_processor/struct.SoftRefProcessing.html\" title=\"struct mmtk::util::reference_processor::SoftRefProcessing\">SoftRefProcessing</a>&lt;E&gt;"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"enum\" href=\"mmtk/util/alloc/allocators/enum.AllocatorSelector.html\" title=\"enum mmtk::util::alloc::allocators::AllocatorSelector\">AllocatorSelector</a>"],["impl&lt;C&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/scheduler/stat/struct.WorkerLocalStat.html\" title=\"struct mmtk::scheduler::stat::WorkerLocalStat\">WorkerLocalStat</a>&lt;C&gt;"],["impl&lt;E: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> + <a class=\"trait\" href=\"mmtk/scheduler/gc_work/trait.ProcessEdgesWork.html\" title=\"trait mmtk::scheduler::gc_work::ProcessEdgesWork\">ProcessEdgesWork</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/reference_processor/struct.PhantomRefProcessing.html\" title=\"struct mmtk::util::reference_processor::PhantomRefProcessing\">PhantomRefProcessing</a>&lt;E&gt;"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/treadmill/struct.TreadMill.html\" title=\"struct mmtk::util::treadmill::TreadMill\">TreadMill</a>"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/util/heap/layout/map64/struct.Map64.html\" title=\"struct mmtk::util::heap::layout::map64::Map64\">Map64</a>"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.71.1/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/scheduler/gc_work/struct.EndOfGC.html\" title=\"struct mmtk::scheduler::gc_work::EndOfGC\">EndOfGC</a>"]]
};if (window.register_implementors) {window.register_implementors(implementors);} else {window.pending_implementors = implementors;}})()