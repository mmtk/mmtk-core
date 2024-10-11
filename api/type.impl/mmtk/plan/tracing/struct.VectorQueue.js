(function() {var type_impls = {
"mmtk":[["<details class=\"toggle implementors-toggle\" open><summary><section id=\"impl-VectorQueue%3CT%3E\" class=\"impl\"><a class=\"src rightside\" href=\"src/mmtk/plan/tracing.rs.html#26-66\">source</a><a href=\"#impl-VectorQueue%3CT%3E\" class=\"anchor\">§</a><h3 class=\"code-header\">impl&lt;T&gt; <a class=\"struct\" href=\"mmtk/plan/tracing/struct.VectorQueue.html\" title=\"struct mmtk::plan::tracing::VectorQueue\">VectorQueue</a>&lt;T&gt;</h3></section></summary><div class=\"impl-items\"><details class=\"toggle\" open><summary><section id=\"associatedconstant.CAPACITY\" class=\"associatedconstant\"><a class=\"src rightside\" href=\"src/mmtk/plan/tracing.rs.html#28\">source</a><h4 class=\"code-header\">const <a href=\"mmtk/plan/tracing/struct.VectorQueue.html#associatedconstant.CAPACITY\" class=\"constant\">CAPACITY</a>: <a class=\"primitive\" href=\"https://doc.rust-lang.org/1.77.0/std/primitive.usize.html\">usize</a> = 4_096usize</h4></section></summary><div class=\"docblock\"><p>Reserve a capacity of this on first enqueue to avoid frequent resizing.</p>\n</div></details><details class=\"toggle method-toggle\" open><summary><section id=\"method.new\" class=\"method\"><a class=\"src rightside\" href=\"src/mmtk/plan/tracing.rs.html#31-33\">source</a><h4 class=\"code-header\">pub fn <a href=\"mmtk/plan/tracing/struct.VectorQueue.html#tymethod.new\" class=\"fn\">new</a>() -&gt; Self</h4></section></summary><div class=\"docblock\"><p>Create an empty <code>VectorObjectQueue</code>.</p>\n</div></details><details class=\"toggle method-toggle\" open><summary><section id=\"method.is_empty\" class=\"method\"><a class=\"src rightside\" href=\"src/mmtk/plan/tracing.rs.html#36-38\">source</a><h4 class=\"code-header\">pub fn <a href=\"mmtk/plan/tracing/struct.VectorQueue.html#tymethod.is_empty\" class=\"fn\">is_empty</a>(&amp;self) -&gt; <a class=\"primitive\" href=\"https://doc.rust-lang.org/1.77.0/std/primitive.bool.html\">bool</a></h4></section></summary><div class=\"docblock\"><p>Return <code>true</code> if the queue is empty.</p>\n</div></details><details class=\"toggle method-toggle\" open><summary><section id=\"method.take\" class=\"method\"><a class=\"src rightside\" href=\"src/mmtk/plan/tracing.rs.html#41-43\">source</a><h4 class=\"code-header\">pub fn <a href=\"mmtk/plan/tracing/struct.VectorQueue.html#tymethod.take\" class=\"fn\">take</a>(&amp;mut self) -&gt; <a class=\"struct\" href=\"https://doc.rust-lang.org/1.77.0/alloc/vec/struct.Vec.html\" title=\"struct alloc::vec::Vec\">Vec</a>&lt;T&gt;</h4></section></summary><div class=\"docblock\"><p>Return the contents of the underlying vector.  It will empty the queue.</p>\n</div></details><details class=\"toggle method-toggle\" open><summary><section id=\"method.into_vec\" class=\"method\"><a class=\"src rightside\" href=\"src/mmtk/plan/tracing.rs.html#46-48\">source</a><h4 class=\"code-header\">pub fn <a href=\"mmtk/plan/tracing/struct.VectorQueue.html#tymethod.into_vec\" class=\"fn\">into_vec</a>(self) -&gt; <a class=\"struct\" href=\"https://doc.rust-lang.org/1.77.0/alloc/vec/struct.Vec.html\" title=\"struct alloc::vec::Vec\">Vec</a>&lt;T&gt;</h4></section></summary><div class=\"docblock\"><p>Consume this <code>VectorObjectQueue</code> and return its underlying vector.</p>\n</div></details><details class=\"toggle method-toggle\" open><summary><section id=\"method.is_full\" class=\"method\"><a class=\"src rightside\" href=\"src/mmtk/plan/tracing.rs.html#51-53\">source</a><h4 class=\"code-header\">pub fn <a href=\"mmtk/plan/tracing/struct.VectorQueue.html#tymethod.is_full\" class=\"fn\">is_full</a>(&amp;self) -&gt; <a class=\"primitive\" href=\"https://doc.rust-lang.org/1.77.0/std/primitive.bool.html\">bool</a></h4></section></summary><div class=\"docblock\"><p>Check if the buffer size reaches <code>CAPACITY</code>.</p>\n</div></details><details class=\"toggle method-toggle\" open><summary><section id=\"method.push\" class=\"method\"><a class=\"src rightside\" href=\"src/mmtk/plan/tracing.rs.html#60-65\">source</a><h4 class=\"code-header\">pub fn <a href=\"mmtk/plan/tracing/struct.VectorQueue.html#tymethod.push\" class=\"fn\">push</a>(&amp;mut self, v: T)</h4></section></summary><div class=\"docblock\"><p>Push an element to the queue. If the queue is empty, it will reserve\nspace to hold the number of elements defined by the capacity.\nThe user of this method needs to make sure the queue length does\nnot exceed the capacity to avoid allocating more space\n(this method will not check the length against the capacity).</p>\n</div></details></div></details>",0,"mmtk::plan::tracing::VectorObjectQueue"],["<details class=\"toggle implementors-toggle\" open><summary><section id=\"impl-Default-for-VectorQueue%3CT%3E\" class=\"impl\"><a class=\"src rightside\" href=\"src/mmtk/plan/tracing.rs.html#68-72\">source</a><a href=\"#impl-Default-for-VectorQueue%3CT%3E\" class=\"anchor\">§</a><h3 class=\"code-header\">impl&lt;T&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.77.0/core/default/trait.Default.html\" title=\"trait core::default::Default\">Default</a> for <a class=\"struct\" href=\"mmtk/plan/tracing/struct.VectorQueue.html\" title=\"struct mmtk::plan::tracing::VectorQueue\">VectorQueue</a>&lt;T&gt;</h3></section></summary><div class=\"impl-items\"><details class=\"toggle method-toggle\" open><summary><section id=\"method.default\" class=\"method trait-impl\"><a class=\"src rightside\" href=\"src/mmtk/plan/tracing.rs.html#69-71\">source</a><a href=\"#method.default\" class=\"anchor\">§</a><h4 class=\"code-header\">fn <a href=\"https://doc.rust-lang.org/1.77.0/core/default/trait.Default.html#tymethod.default\" class=\"fn\">default</a>() -&gt; Self</h4></section></summary><div class='docblock'>Returns the “default value” for a type. <a href=\"https://doc.rust-lang.org/1.77.0/core/default/trait.Default.html#tymethod.default\">Read more</a></div></details></div></details>","Default","mmtk::plan::tracing::VectorObjectQueue"],["<details class=\"toggle implementors-toggle\" open><summary><section id=\"impl-ObjectQueue-for-VectorQueue%3CObjectReference%3E\" class=\"impl\"><a class=\"src rightside\" href=\"src/mmtk/plan/tracing.rs.html#74-78\">source</a><a href=\"#impl-ObjectQueue-for-VectorQueue%3CObjectReference%3E\" class=\"anchor\">§</a><h3 class=\"code-header\">impl <a class=\"trait\" href=\"mmtk/plan/tracing/trait.ObjectQueue.html\" title=\"trait mmtk::plan::tracing::ObjectQueue\">ObjectQueue</a> for <a class=\"struct\" href=\"mmtk/plan/tracing/struct.VectorQueue.html\" title=\"struct mmtk::plan::tracing::VectorQueue\">VectorQueue</a>&lt;<a class=\"struct\" href=\"mmtk/util/address/struct.ObjectReference.html\" title=\"struct mmtk::util::address::ObjectReference\">ObjectReference</a>&gt;</h3></section></summary><div class=\"impl-items\"><details class=\"toggle method-toggle\" open><summary><section id=\"method.enqueue\" class=\"method trait-impl\"><a class=\"src rightside\" href=\"src/mmtk/plan/tracing.rs.html#75-77\">source</a><a href=\"#method.enqueue\" class=\"anchor\">§</a><h4 class=\"code-header\">fn <a href=\"mmtk/plan/tracing/trait.ObjectQueue.html#tymethod.enqueue\" class=\"fn\">enqueue</a>(&amp;mut self, v: <a class=\"struct\" href=\"mmtk/util/address/struct.ObjectReference.html\" title=\"struct mmtk::util::address::ObjectReference\">ObjectReference</a>)</h4></section></summary><div class='docblock'>Enqueue an object into the queue.</div></details></div></details>","ObjectQueue","mmtk::plan::tracing::VectorObjectQueue"]]
};if (window.register_type_impls) {window.register_type_impls(type_impls);} else {window.pending_type_impls = type_impls;}})()