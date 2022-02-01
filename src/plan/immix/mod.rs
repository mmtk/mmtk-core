// pub(super) mod gc_work;
pub(crate) mod gc_work_original;
pub(crate) mod gc_work_general;
pub(crate) mod gc_work_opt;

pub(crate) use gc_work_opt as gc_work;

pub(super) mod global;
pub(super) mod mutator;

pub use self::global::Immix;
pub use self::global::IMMIX_CONSTRAINTS;
