use super::{G1MarkTraceLocal, G1EvacuateTraceLocal, G1NurseryTraceLocal};
use super::multitracelocal::*;
use super::validate::{self, Validator};
use super::PLAN;
use policy::region::*;
use ::util::{Address, ObjectReference};
use policy::space::Space;

#[repr(u8)]
#[derive(PartialEq, Eq)]
pub enum TraceKind {
    Mark = 0,
    Evacuate = 1,
    Nursery = 2,
    Validate = 3,
}

fn spaceof(o: ObjectReference) -> &'static str {
    if PLAN.vm_space.in_space(o) {
        "vm"
    } else if PLAN.versatile_space.in_space(o) {
        "vs"
    } else if PLAN.region_space.in_space(o) {
        "g1"
    } else if PLAN.los.in_space(o) {
        "los"
    } else {
        unreachable!()
    }
}



impl Validator for () {
    fn validate_edge(src: ObjectReference, slot: Address, obj: ObjectReference) {
        // assert!(PLAN.is_mapped_object(src));
        if obj.is_null() {
            return
        }
        if !PLAN.is_mapped_object(obj) {
            println!("<{} {:?}>.{:?} points to an unmapped object {:?}", spaceof(src), src, slot, obj)
        }
        assert!(PLAN.is_mapped_object(obj));
        
        if PLAN.region_space.in_space(obj) {
            let region = Region::of_object(obj);
            assert!(region.committed,
                "{} {:?} {:?}.{:?} -> {} {:?}", Card::of(src).0, spaceof(src), src, slot, spaceof(obj), obj
            );
            assert!(!region.relocate,
                "{} {:?}.{:?} -> {} {:?}", spaceof(src), src, slot, spaceof(obj), obj
            );
            if region != Region::of_object(src) && !PLAN.vm_space.in_space(src) {
                if Card::of(src).get_state() == CardState::Dirty {
                    if !region.remset().contains_card(Card::of(src)) {
                        // println!(
                        //     "WARNING: {} card {:?} for object {:?}, slot {:?} is not remembered by {} region {:?} ({:?})", spaceof(src), Card::of(src).0, src, slot, spaceof(obj), region, obj
                        // );
                    }
                } else {
                    assert!(region.remset().contains_card(Card::of(src)),
                        "{} card {:?} for object {:?}, slot {:?} is not remembered by {} region {:?} ({:?})", spaceof(src), Card::of(src).0, src, slot, spaceof(obj), region, obj
                    )
                }
            }
        }
    }
    
    fn validate_object(o: ObjectReference) {
        if o.is_null() {
            return
        }
        // }
        // if !PLAN.is_mapped_object(obj) {
        //     println!("<{} {:?}>.{:?} points to an unmapped object {:?}", get_space_name(src), src, slot, obj)
        // }
        assert!(PLAN.is_mapped_object(o));
        // assert!(PLAN.is_mapped_object(o));
        if PLAN.region_space.in_space(o) {
            let region = Region::of_object(o);
            assert!(region.committed);
            assert!(!region.relocate);
        } else if PLAN.versatile_space.in_space(o) {
            assert!(PLAN.versatile_space.is_marked(o));
        } else if PLAN.los.in_space(o) {
            assert!(PLAN.los.is_live(o));
        } else if PLAN.vm_space.in_space(o) {
            assert!(PLAN.vm_space.is_marked(o), "{:?} is not marked", o);
        } else {
            panic!("Unmapped object {:?}", o)
        }
    }
}

#[allow(dead_code)]
fn in_regions_set(r: Region) -> bool {
    for x in PLAN.region_space.regions() {
        if x == r {
            return true
        }
    }
    return false
}

type ValidateTraceLocal = validate::ValidateTraceLocal<()>;

pub type G1TraceLocal = Cons<G1MarkTraceLocal, Cons<G1EvacuateTraceLocal, Cons<G1NurseryTraceLocal, Cons<ValidateTraceLocal, Nil>>>>;

impl G1TraceLocal {
    pub fn mark_trace(&self) -> &G1MarkTraceLocal {
        &self.head
    }
    pub fn mark_trace_mut(&mut self) -> &mut G1MarkTraceLocal {
        &mut self.head
    }
    pub fn evacuate_trace(&self) -> &G1EvacuateTraceLocal {
        &self.tail.head
    }
    pub fn evacuate_trace_mut(&mut self) -> &mut G1EvacuateTraceLocal {
        &mut self.tail.head
    }
    pub fn nursery_trace(&self) -> &G1NurseryTraceLocal {
        &self.tail.tail.head
    }
    pub fn nursery_trace_mut(&mut self) -> &mut G1NurseryTraceLocal {
        &mut self.tail.tail.head
    }
    pub fn validate_trace_mut(&mut self) -> &mut ValidateTraceLocal {
        &mut self.tail.tail.tail.head
    }
    pub fn activated_trace(&self) -> TraceKind {
        if self.active {
            TraceKind::Mark
        } else if self.tail.active {
            TraceKind::Evacuate
        } else if self.tail.tail.active {
            TraceKind::Validate
        } else {
            unreachable!()
        }
    }
}