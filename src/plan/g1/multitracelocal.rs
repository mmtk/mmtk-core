use ::plan::{TraceLocal, TransitiveClosure};
use ::util::{Address, ObjectReference};


/**
 * type G1TraceLocal = Cons<MarkTraceLocal, Cons<EvacuateTraceLocal, Nil>>
 */

pub trait Node: Sized + TransitiveClosure + TraceLocal {
    fn set_active(&mut self, index: u8);
    fn set_all_inactive(&mut self);
    fn activated_index(&self) -> u8;
}

pub struct Cons<H: TraceLocal, T: Node> {
    pub active: bool,
    pub head: H,
    pub tail: T,
}
pub struct Nil;

impl <H: TraceLocal, T: Node> Node for Cons<H, T> {
    fn set_all_inactive(&mut self) {
        self.active = false;
        self.tail.set_all_inactive();
    }
    fn set_active(&mut self, index: u8) {
        if index == 0 {
            self.active = true;
            self.tail.set_all_inactive();
        } else {
            self.active = false;
            self.tail.set_active(index - 1);
        }
    }
    fn activated_index(&self) -> u8 {
        if self.active {
            0
        } else {
            1 + self.tail.activated_index()
        }
    }
}

impl Node for Nil {
    fn set_all_inactive(&mut self) {}
    fn set_active(&mut self, _index: u8) {
        unreachable!()
    }
    fn activated_index(&self) -> u8 {
        unreachable!()
    }
}

/**
 * let trace_local = multitracelocal!(MarkTraceLocal::new(), EvacuateTraceLocal::new());
 */

impl <H: TraceLocal, T: Node> Cons<H, T> {
    pub fn new(head: H, tail: T) -> Self {
        Self { active: false, head, tail }
    }
}

#[macro_export]
macro_rules! multitracelocal {
    ($h:expr, $($t:expr),*) => (Cons::new($h, multitracelocal!($($t),*)));
    ($h:expr) => (Cons::new($h, Nil));
    () => (Nil);
}

#[macro_export]
macro_rules! type_list {
    ($h:ty, $($t:ty,)*) => (Cons<$ty, type_list!($($t,)*)>);
    ($h:ty) => (Cons<$h, Nil>);
    () => (Nil);
}


impl <H: TraceLocal, T: Node> TransitiveClosure for Cons<H, T> {
    #[inline(always)]
    fn process_edge(&mut self, src: ObjectReference, slot: Address) {
        if self.active {
            self.head.process_edge(src, slot)
        } else {
            self.tail.process_edge(src, slot)
        }
    }
    #[inline(always)]
    fn process_node(&mut self, object: ObjectReference) {
        if self.active {
            self.head.process_node(object)
        } else {
            self.tail.process_node(object)
        }
    }
}

impl <H: TraceLocal, T: Node> TraceLocal for Cons<H, T> {
    #[inline(always)]
    fn process_remembered_sets(&mut self) {
        if self.active {
            self.head.process_remembered_sets()
        } else {
            self.tail.process_remembered_sets()
        }
    }
    #[inline(always)]
    fn process_roots(&mut self) {
        if self.active {
            self.head.process_roots()
        } else {
            self.tail.process_roots()
        }
    }
    #[inline(always)]
    fn process_root_edge(&mut self, slot: Address, untraced: bool) {
        if self.active {
            self.head.process_root_edge(slot, untraced)
        } else {
            self.tail.process_root_edge(slot, untraced)
        }
    }
    #[inline(always)]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if self.active {
            self.head.trace_object(object)
        } else {
            self.tail.trace_object(object)
        }
    }
    #[inline(always)]
    fn complete_trace(&mut self) {
        if self.active {
            self.head.complete_trace()
        } else {
            self.tail.complete_trace()
        }
    }
    #[inline(always)]
    fn release(&mut self) {
        if self.active {
            self.head.release()
        } else {
            self.tail.release()
        }
    }
    #[inline(always)]
    fn process_interior_edge(&mut self, target: ObjectReference, slot: Address, root: bool) {
        if self.active {
            self.head.process_interior_edge(target, slot, root)
        } else {
            self.tail.process_interior_edge(target, slot, root)
        }
    }
    #[inline(always)]
    fn report_delayed_root_edge(&mut self, slot: Address) {
        if self.active {
            self.head.report_delayed_root_edge(slot)
        } else {
            self.tail.report_delayed_root_edge(slot)
        }
    }
    #[inline(always)]
    fn will_not_move_in_current_collection(&self, obj: ObjectReference) -> bool {
        if self.active {
            self.head.will_not_move_in_current_collection(obj)
        } else {
            self.tail.will_not_move_in_current_collection(obj)
        }
    }
    #[inline(always)]
    fn is_live(&self, object: ObjectReference) -> bool {
        if self.active {
            self.head.is_live(object)
        } else {
            self.tail.is_live(object)
        }
    }
    #[inline(always)]
    fn overwrite_reference_during_trace(&self) -> bool {
        if self.active {
            self.head.overwrite_reference_during_trace()
        } else {
            self.tail.overwrite_reference_during_trace()
        }
    }
    #[inline(always)]
    fn get_forwarded_reference(&mut self, object: ObjectReference) -> ObjectReference {
        if self.active {
            self.head.get_forwarded_reference(object)
        } else {
            self.tail.get_forwarded_reference(object)
        }
    }
    #[inline(always)]
    fn get_forwarded_referent(&mut self, object: ObjectReference) -> ObjectReference {
        if self.active {
            self.head.get_forwarded_referent(object)
        } else {
            self.tail.get_forwarded_referent(object)
        }
    }
    #[inline(always)]
    fn retain_referent(&mut self, object: ObjectReference) -> ObjectReference {
           if self.active {
            self.head.retain_referent(object)
        } else {
            self.tail.retain_referent(object)
        }
    }
}

impl TransitiveClosure for Nil {
    fn process_edge(&mut self, _: ObjectReference, _: Address) {
        unreachable!()
    }
    fn process_node(&mut self, _: ObjectReference) {
        unreachable!()
    }
}

impl TraceLocal for Nil {
    fn process_remembered_sets(&mut self) {
        unreachable!()
    }
    fn process_roots(&mut self) {
        unreachable!()
    }
    fn process_root_edge(&mut self, _: Address, _: bool) {
        unreachable!()
    }
    fn trace_object(&mut self, _: ObjectReference) -> ObjectReference {
        unreachable!()
    }
    fn complete_trace(&mut self) {
        unreachable!()
    }
    fn release(&mut self) {
        unreachable!()
    }
    fn process_interior_edge(&mut self, _: ObjectReference, _: Address, _: bool) {
        unreachable!()
    }
    fn report_delayed_root_edge(&mut self, _: Address) {
        unreachable!()
    }
    fn will_not_move_in_current_collection(&self, _: ObjectReference) -> bool {
        unreachable!()
    }
    fn is_live(&self, _: ObjectReference) -> bool {
        unreachable!()
    }
}
