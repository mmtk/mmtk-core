use std::fmt::Debug;
use std::mem::MaybeUninit;
use std::{env, str::FromStr};

#[derive(Debug)]
pub(crate) struct RuntimeArgs {
    pub(crate) incs_limit: Option<usize>,
    pub(crate) nursery_blocks: Option<usize>,
    pub(crate) young_limit_mb: Option<usize>,
    pub(crate) nursery_ratio: Option<usize>,
    pub(crate) lower_concurrent_worker_priority: bool,
    #[allow(unused)]
    pub(crate) max_mature_defrag_percent: usize,
    pub(crate) max_pause_millis: Option<usize>,
    /// Terminate CM or RC loop if the availabel heap after a RC pause is still small
    pub(crate) rc_stop_percent: usize,
    pub(crate) max_survival_mb: usize,
    pub(crate) trace_threshold: usize,
}

impl Default for RuntimeArgs {
    fn default() -> Self {
        fn env_arg<T: FromStr + Debug>(name: &str) -> Option<T>
        where
            T::Err: Debug,
        {
            env::var(name).map(|x| T::from_str(&x).unwrap()).ok()
        }
        Self {
            incs_limit: env_arg("INCS_LIMIT"),
            nursery_blocks: env_arg("NURSERY_BLOCKS"),
            young_limit_mb: env_arg("YOUNG_LIMIT").or_else(|| env_arg("YOUNG_LIMIT_MB")),
            nursery_ratio: env_arg("NURSERY_RATIO"),
            lower_concurrent_worker_priority: env_arg("LOWER_CONCURRENT_WORKER_PRIORITY")
                .unwrap_or(false),
            max_mature_defrag_percent: env_arg("MAX_MATURE_DEFRAG_PERCENT").unwrap_or(15),
            max_pause_millis: env_arg("MAX_PAUSE_MILLIS"),
            rc_stop_percent: env_arg("RC_STOP_PERCENT").unwrap_or(15),
            max_survival_mb: env_arg::<usize>("MAX_SURVIVAL_MB").unwrap_or(128),
            trace_threshold: env_arg("TRACE_THRESHOLD2")
                .or_else(|| env_arg("TRACE_THRESHOLD"))
                .or_else(|| env_arg("CM_THRESHOLD"))
                .unwrap_or(20),
        }
    }
}

static mut ARGS: MaybeUninit<RuntimeArgs> = MaybeUninit::uninit();

impl RuntimeArgs {
    pub fn init() {
        unsafe {
            ARGS.write(RuntimeArgs::default());
        }
    }
    pub fn get() -> &'static Self {
        unsafe { &*ARGS.as_ptr() }
    }
}

pub const BUFFER_SIZE: usize = 1024;

pub const CYCLE_TRIGGER_THRESHOLD: usize = 1024;

// ---------- CM/RC Immix flags ---------- //
pub const LAZY_DECREMENTS: bool = !cfg!(feature = "lxr_no_lazy");
pub const RC_NURSERY_EVACUATION: bool = !cfg!(feature = "lxr_no_nursery_evac");
pub const RC_MATURE_EVACUATION: bool = !cfg!(feature = "lxr_no_mature_evac");

