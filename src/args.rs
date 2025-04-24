use crate::{
    policy::immix::{block::Block, line::Line},
    util::{
        constants::LOG_BYTES_IN_MBYTE, heap::vm_layout::vm_layout, linear_scan::Region,
        options::Options,
    },
    BarrierSelector,
};
use std::fmt::Debug;
use std::mem::MaybeUninit;
use std::{env, str::FromStr};

#[derive(Debug)]
pub(crate) struct RuntimeArgs {
    pub(crate) incs_limit: Option<usize>,
    pub(crate) no_mutator_line_recycling: bool,
    pub(crate) no_line_recycling: bool,
    pub(crate) nursery_blocks: Option<usize>,
    pub(crate) young_limit_mb: Option<usize>,
    pub(crate) nursery_ratio: Option<usize>,
    pub(crate) lower_concurrent_worker_priority: bool,
    #[allow(unused)]
    pub(crate) max_mature_defrag_percent: usize,
    pub(crate) max_pause_millis: Option<usize>,
    pub(crate) max_young_evac_size: usize,
    /// Terminate CM or RC loop if the availabel heap after a RC pause is still small
    pub(crate) rc_stop_percent: usize,
    pub(crate) max_survival_mb: usize,
    pub(crate) survival_predictor_harmonic_mean: bool,
    pub(crate) survival_predictor_weighted: bool,
    pub(crate) trace_threshold: usize,
    pub(crate) min_reuse_lines: usize,
    pub(crate) chunk_defarg_percent: usize,
    pub(crate) transparent_hugepage: bool,
}

impl Default for RuntimeArgs {
    fn default() -> Self {
        fn env_arg<T: FromStr + Debug>(name: &str) -> Option<T>
        where
            T::Err: Debug,
        {
            env::var(name).map(|x| T::from_str(&x).unwrap()).ok()
        }
        fn env_bool_arg(name: &str) -> Option<bool> {
            env::var(name)
                .map(|x| x == "1" || x == "true" || x == "TRUE")
                .ok()
        }
        Self {
            incs_limit: env_arg("INCS_LIMIT"),
            no_mutator_line_recycling: env_bool_arg("NO_MUTATOR_LINE_RECYCLING").unwrap_or(false),
            no_line_recycling: env_bool_arg("NO_LINE_RECYCLING").unwrap_or(false),
            nursery_blocks: env_arg("NURSERY_BLOCKS").or(
                if cfg!(feature = "lxr_fixed_clean_rc_trigger") {
                    const BLOCKS_IN_MB: usize = (1 << 20) >> Block::LOG_BYTES;
                    Some(128 * BLOCKS_IN_MB) // 128 M
                } else {
                    None
                },
            ),
            young_limit_mb: env_arg("YOUNG_LIMIT")
                .or_else(|| env_arg("YOUNG_LIMIT_MB"))
                .or(if cfg!(feature = "lxr_fixed_rc_trigger") {
                    Some(128 << LOG_BYTES_IN_MBYTE) // 128 M
                } else {
                    None
                }),
            nursery_ratio: env_arg("NURSERY_RATIO"),
            lower_concurrent_worker_priority: env_arg("LOWER_CONCURRENT_WORKER_PRIORITY")
                .unwrap_or(false),
            max_mature_defrag_percent: env_arg("MAX_MATURE_DEFRAG_PERCENT").unwrap_or(15),
            max_pause_millis: env_arg("MAX_PAUSE_MILLIS"),
            max_young_evac_size: env_arg("MAX_YOUNG_EVAC_SIZE").unwrap_or(usize::MAX),
            rc_stop_percent: env_arg("RC_STOP_PERCENT").unwrap_or(15),
            max_survival_mb: if cfg!(feature = "lxr_fixed_rc_trigger")
                || cfg!(feature = "lxr_fixed_clean_rc_trigger")
            {
                usize::MAX
            } else {
                env_arg::<usize>("MAX_SURVIVAL_MB").unwrap_or(128)
            },
            survival_predictor_harmonic_mean: env_bool_arg("SURVIVAL_PREDICTOR_HARMONIC_MEAN")
                .unwrap_or(false),
            survival_predictor_weighted: env_bool_arg("SURVIVAL_PREDICTOR_WEIGHTED")
                .unwrap_or(false),
            trace_threshold: env_arg("TRACE_THRESHOLD2")
                .or_else(|| env_arg("TRACE_THRESHOLD"))
                .or_else(|| env_arg("CM_THRESHOLD"))
                .unwrap_or(if cfg!(feature = "lxr_simple_satb_trigger") {
                    50
                } else {
                    20
                }),
            min_reuse_lines: env_arg::<usize>("MIN_REUSE_LINES").unwrap_or(1),
            chunk_defarg_percent: env_arg::<usize>("CHUNK_DEFARG_THRESHOLD").unwrap_or(32),
            transparent_hugepage: env_bool_arg("TRANSPARENT_HUGEPAGE")
                .or(env_bool_arg("HUGEPAGE"))
                .unwrap_or(true),
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

pub const CM_LARGE_ARRAY_OPTIMIZATION: bool = false;
pub const BUFFER_SIZE: usize = {
    if cfg!(feature = "lxr_buf_2048") {
        2048
    } else if cfg!(feature = "lxr_buf_1024") {
        1024
    } else if cfg!(feature = "lxr_buf_512") {
        512
    } else if cfg!(feature = "lxr_buf_256") {
        256
    } else {
        1024
    }
};

pub const HOLE_COUNTING: bool = cfg!(feature = "lxr_hole_counting");
pub const NO_LAZY_SWEEP_WHEN_STW_CANNOT_RELEASE_ENOUGH_MEMORY: bool = false;

// ---------- Immix flags ---------- //
pub const CYCLE_TRIGGER_THRESHOLD: usize = 1024;
/// Mark/sweep memory for block-level only
pub const BLOCK_ONLY: bool = cfg!(feature = "ix_block_only");
/// Opportunistic copying
pub const DEFRAG: bool = !cfg!(feature = "ix_no_defrag");
/// Mark lines when scanning objects. Otherwise, do it at mark time.
pub const MARK_LINE_AT_SCAN_TIME: bool = true;

// ---------- CM/RC Immix flags ---------- //
pub const EAGER_INCREMENTS: bool = false;
pub const LAZY_DECREMENTS: bool = !cfg!(feature = "lxr_no_lazy");
pub const NO_LAZY_DEC_THRESHOLD: usize = 100;
pub const RC_NURSERY_EVACUATION: bool = !cfg!(feature = "lxr_no_nursery_evac");
pub const RC_MATURE_EVACUATION: bool = !cfg!(feature = "lxr_no_mature_evac");
pub const ENABLE_INITIAL_ALLOC_LIMIT: bool = cfg!(feature = "lxr_enable_initial_alloc_limit");

/// One more atomic-store per barrier slow-path if this value is smaller than 6.
pub const LOG_BYTES_PER_RC_LOCK_BIT: usize = {
    if cfg!(feature = "lxr_lock_3") {
        3
    } else if cfg!(feature = "lxr_lock_4") {
        4
    } else if cfg!(feature = "lxr_lock_5") {
        5
    } else if cfg!(feature = "lxr_lock_6") {
        6
    } else if cfg!(feature = "lxr_lock_7") {
        7
    } else if cfg!(feature = "lxr_lock_8") {
        8
    } else if cfg!(feature = "lxr_lock_9") {
        9
    } else {
        9
    }
};
pub const RC_DONT_EVACUATE_NURSERY_IN_RECYCLED_LINES: bool =
    !cfg!(feature = "lxr_evacuate_nursery_in_recycled_lines");

// ---------- Barrier flags ---------- //
pub const BARRIER_MEASUREMENT: bool = cfg!(feature = "barrier_measurement");
pub const BARRIER_MEASUREMENT_NO_SLOW: bool = cfg!(feature = "barrier_measurement_no_slow");
pub const TAKERATE_MEASUREMENT: bool = false;
pub const INSTRUMENTATION: bool = cfg!(feature = "instrumentation");

// ---------- Debugging flags ---------- //
pub const HARNESS_PRETTY_PRINT: bool = false || cfg!(feature = "log_gc");
pub const LOG_WORK_PACKETS: bool = cfg!(feature = "log_work_packets");
pub const NO_RC_PAUSES_DURING_CONCURRENT_MARKING: bool = cfg!(feature = "lxr_no_rc_in_cm");
pub const SLOW_CONCURRENT_MARKING: bool = false;
pub const INC_MAX_COPY_DEPTH: bool = false;
pub const PREFETCH: bool = !cfg!(feature = "lxr_no_prefetch");
pub const PREFETCH_HEADER: bool = PREFETCH;
pub const PREFETCH_MARK: bool = PREFETCH;
pub const PREFETCH_RC: bool = false;
pub const PREFETCH_STEP: usize = 8;

macro_rules! dump_feature {
    ($name: literal, $value: expr) => {
        eprintln!(" * {}: {:?}", $name, $value)
    };
    ($name: literal) => {
        dump_feature!($name, cfg!(feature = $name))
    };
}

fn dump_features(active_barrier: BarrierSelector, options: &Options) {
    if *options.verbose == 0 {
        return;
    }
    eprintln!("JVM Process ID: {}", std::process::id());
    eprintln!("-------------------- Immix Args --------------------");

    dump_feature!("barrier", format!("{:?}", active_barrier));
    dump_feature!("barrier_measurement");
    dump_feature!("instrumentation");
    dump_feature!("ix_block_only");
    dump_feature!("ix_no_defrag");
    dump_feature!("lxr_evacuate_nursery_in_recycled_lines");
    dump_feature!("lxr_delayed_nursery_evacuation");
    dump_feature!("lxr_enable_initial_alloc_limit");
    dump_feature!("log_block_size", Block::LOG_BYTES);
    dump_feature!("log_line_size", Line::LOG_BYTES);
    dump_feature!("log_bytes_per_rc_lock_bit", LOG_BYTES_PER_RC_LOCK_BIT);
    dump_feature!("buffer_size", BUFFER_SIZE);
    dump_feature!("nontemporal");
    dump_feature!("cm_large_array_optimization", CM_LARGE_ARRAY_OPTIMIZATION);
    dump_feature!("inc_max_copy_depth", INC_MAX_COPY_DEPTH);
    dump_feature!("no_finalizer", *options.no_finalizer);
    dump_feature!("no_reference_types", *options.no_reference_types);
    dump_feature!("workers", *options.threads);
    dump_feature!("conc_workers", *options.conc_threads);
    dump_feature!(
        "force_use_contiguous_spaces",
        vm_layout().force_use_contiguous_spaces
    );
    dump_feature!("bpr_spin_lock");
    dump_feature!("lxr_no_nursery_evac");
    dump_feature!("lxr_fixed_young_size");
    dump_feature!("lxr_no_lazy_young_sweeping");
    dump_feature!("lxr_no_chunk_defrag");
    dump_feature!("lxr_no_lazy");
    dump_feature!("lxr_no_cm");
    dump_feature!("lxr_srv_ratio_counter");
    dump_feature!("lxr_release_stage_timer");
    dump_feature!("lxr_precise_incs_counter");
    dump_feature!("lxr_fixed_satb_trigger");
    dump_feature!("lxr_no_survival_trigger");
    dump_feature!("no_map_fixed_noreplace");
    dump_feature!("report_worker_sleep_events");
    dump_feature!("no_meta_counting");
    dump_feature!("lxr_no_srv_copy_reserve");
    dump_feature!("lxr_abort_on_trace");
    dump_feature!("lxr_simple_satb_trigger");
    dump_feature!("measure_trace_rate");
    dump_feature!("lxr_no_mature_defrag");

    eprintln!("\n{:#?}", RuntimeArgs::get());

    eprintln!("----------------------------------------------------");
}

pub fn validate_features(active_barrier: BarrierSelector, options: &Options) {
    dump_features(active_barrier, options);
    validate!(DEFRAG => !BLOCK_ONLY);
    validate!(EAGER_INCREMENTS => !RC_NURSERY_EVACUATION);
    validate!(RC_NURSERY_EVACUATION => !EAGER_INCREMENTS);
}
