use crate::plan::Plan;
use crate::plan::SelectedPlan;

use std::sync::Arc;

// TODO: remove this singleton at some point to allow multiple instances of MMTK
// This helps refactoring.
lazy_static!{
    pub static ref SINGLETON: MMTK = MMTK::new();
}

pub struct MMTK {
    pub plan: SelectedPlan,
}

impl MMTK {
    pub fn new() -> Self {
        MMTK {
            plan: SelectedPlan::new(),
        }
    }
}