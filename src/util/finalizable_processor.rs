use crate::util::ObjectReference;
use crate::plan::TraceLocal;

pub struct FinalizableProcessor {
    candidates: Vec<ObjectReference>,
    nursery_index: usize,
    ready_for_finalize: Vec<ObjectReference>,
}

impl FinalizableProcessor {
    pub fn new() -> Self {
        Self {
            candidates: vec![],
            nursery_index: 0,
            ready_for_finalize: vec![],
        }
    }

    pub fn add(&mut self, object: ObjectReference) {
        self.candidates.push(object);
    }

    pub fn scan<T: TraceLocal>(&mut self, trace: &mut T, nursery: bool) {
        let start = if nursery {
            self.nursery_index
        } else {
            0
        };

        for reff in self.candidates.drain(start..).collect::<Vec<ObjectReference>>() {
            if reff.is_live() {
                self.candidates.push(trace.get_forwarded_finalizable(reff));
                continue;
            }

            let retained = trace.return_for_finalize(reff);
            self.ready_for_finalize.push(retained);
        }

        self.nursery_index = self.candidates.len();
    }

    pub fn forward<T: TraceLocal>(&mut self, trace: &mut T, _nursery: bool) {
        self.candidates.iter_mut().for_each(|reff| *reff = trace.get_forwarded_finalizable(*reff));
    }

    pub fn get_ready_object(&mut self) -> Option<ObjectReference> {
        self.ready_for_finalize.pop()
    }
}
