use crate::util::Address;
use std::collections::HashMap;
use std::io::{Error, ErrorKind, Result};
use std::sync::RwLock;

use super::constants::LOG_GLOBAL_SIDE_METADATA_WORST_CASE_RATIO;
use super::constants::LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO;
use super::{SideMetadataContext, SideMetadataSpec};
use crate::util::heap::layout::vm_layout_constants::LOG_ADDRESS_SPACE;
#[cfg(target_pointer_width = "32")]
use crate::util::heap::layout::vm_layout_constants::LOG_BYTES_IN_CHUNK;

#[cfg(feature = "extreme_assertions")]
enum MathOp {
    Add,
    Sub,
}

lazy_static! {
    static ref SANITY_MAP: RwLock<Vec<HashMap<Address, usize>>> = RwLock::new(vec![]);
    static ref SPEC_TO_IDX_MAP: RwLock<HashMap<SideMetadataSpec, usize>> =
        RwLock::new(HashMap::new());
}

#[cfg(feature = "extreme_assertions")]
fn spec_to_index(metadata_spec: &SideMetadataSpec) -> Option<usize> {
    let spec_to_index_map = SPEC_TO_IDX_MAP.read().unwrap();
    match spec_to_index_map.get(metadata_spec) {
        Some(idx) => Some(*idx),
        None => None,
    }
}

fn verify_global_specs_total_size(g_specs: &[SideMetadataSpec]) -> Result<()> {
    let mut total_size = 0usize;
    for spec in g_specs {
        total_size += super::metadata_address_range_size(*spec);
    }

    if total_size <= 1usize << (LOG_ADDRESS_SPACE - LOG_GLOBAL_SIDE_METADATA_WORST_CASE_RATIO) {
        Ok(())
    } else {
        Err(Error::new(
            ErrorKind::InvalidInput,
            format!("Not enough global metadata space for: \n{:?}", g_specs),
        ))
    }
}

#[cfg(target_pointer_width = "64")]
fn verify_local_specs_size(l_specs: &[SideMetadataSpec]) -> Result<()> {
    for spec in l_specs {
        if super::metadata_address_range_size(*spec)
            > 1usize << (LOG_ADDRESS_SPACE - LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO)
        {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("Local metadata is too big: \n{:?}", spec),
            ));
        }
    }

    Ok(())
}

#[cfg(target_pointer_width = "32")]
fn verify_local_specs_size(l_specs: &[SideMetadataSpec]) -> Result<()> {
    let mut total_size = 0usize;
    for spec in l_specs {
        total_size += super::meta_bytes_per_chunk(spec.log_min_obj_size, spec.log_num_of_bits);
    }

    if total_size > 1usize << (LOG_BYTES_IN_CHUNK - LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("Not local metadata space per chunk for: \n{:?}", l_specs),
        ));
    }

    Ok(())
}

fn verify_no_overlap_contiguous(
    spec_1: &SideMetadataSpec,
    spec_2: &SideMetadataSpec,
) -> Result<()> {
    let end_1 = spec_1.offset + super::metadata_address_range_size(*spec_1);
    let end_2 = spec_2.offset + super::metadata_address_range_size(*spec_2);

    if !(spec_1.offset >= end_2 || spec_2.offset >= end_1) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "Overlapping metadata specs detected:\nTHIS:\n{:#?}\nAND:\n{:#?}",
                spec_1, spec_2
            ),
        ));
    }
    Ok(())
}

#[cfg(target_pointer_width = "32")]
fn verify_no_overlap_chunked(spec_1: &SideMetadataSpec, spec_2: &SideMetadataSpec) -> Result<()> {
    let end_1 = spec_1.offset
        + super::meta_bytes_per_chunk(spec_1.log_min_obj_size, spec_1.log_num_of_bits);
    let end_2 = spec_2.offset
        + super::meta_bytes_per_chunk(spec_2.log_min_obj_size, spec_2.log_num_of_bits);

    if !(spec_1.offset >= end_2 || spec_2.offset >= end_1) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "Overlapping metadata specs detected:\nTHIS:\n{:#?}\nAND:\n{:#?}",
                spec_1, spec_2
            ),
        ));
    }
    Ok(())
}

fn verify_global_specs(g_specs: &[SideMetadataSpec]) -> Result<()> {
    let v = verify_global_specs_total_size(g_specs);
    if v.is_err() {
        return v;
    }

    for spec_1 in g_specs {
        for spec_2 in g_specs {
            if spec_1 != spec_2 {
                let v = verify_no_overlap_contiguous(spec_1, spec_2);
                if v.is_err() {
                    return v;
                }
            }
        }
    }

    Ok(())
}

fn get_all_specs(global: bool) -> Vec<SideMetadataSpec> {
    let mut specs = vec![];
    let idx_map = SPEC_TO_IDX_MAP.read().unwrap();
    for (k, _) in idx_map.iter() {
        if !(global ^ k.scope.is_global()) {
            specs.push(*k);
        }
    }

    specs
}

fn verify_local_specs() -> Result<()> {
    let local_specs = get_all_specs(false);

    let v = verify_local_specs_size(&local_specs);
    if v.is_err() {
        return v;
    }

    for spec_1 in &local_specs {
        for spec_2 in &local_specs {
            if spec_1 != spec_2 {
                #[cfg(target_pointer_width = "64")]
                let v = verify_no_overlap_contiguous(spec_1, spec_2);
                #[cfg(target_pointer_width = "32")]
                let v = verify_no_overlap_chunked(spec_1, spec_2);
                if v.is_err() {
                    return v;
                }
            }
        }
    }
    Ok(())
}

#[allow(dead_code)]
pub fn reset() {
    let mut sanity_map = SANITY_MAP.write().unwrap();
    let mut idx_map = SPEC_TO_IDX_MAP.write().unwrap();
    sanity_map.clear();
    idx_map.clear();
}

pub fn verify_metadata_context(metadata_context: &SideMetadataContext) -> Result<()> {
    // global metadata combination is the same for all contexts
    let v = verify_global_specs(&metadata_context.global);
    if v.is_err() {
        return v;
    }

    // assert not initialised before
    let mut sanity_map = SANITY_MAP.write().unwrap();
    let mut idx_map = SPEC_TO_IDX_MAP.write().unwrap();

    let global_count = metadata_context.global.len();
    let local_count = metadata_context.local.len();

    // println!("check_metadata_context.g({}).l({})", global_count, local_count);

    let cur_total_count = sanity_map.len();
    let first_call = cur_total_count == 0;
    // sanity_map.reserve(global_count + local_count);

    for i in 0..global_count {
        let spec = metadata_context.global[i];
        if first_call {
            // initialise the related hashmap
            sanity_map.push(HashMap::new());
            // add this metadata to index map
            idx_map.insert(spec, i);
        } else if !idx_map.contains_key(&spec) {
            drop(idx_map);
            drop(sanity_map);
            return Err(Error::new(ErrorKind::InvalidInput, format!("Global metadata must not change between policies! NEW SPEC: {:#?} OLD SPECS: {:#?}", spec, get_all_specs(true))));
        }
    }

    for i in 0..local_count {
        if !idx_map.contains_key(&metadata_context.local[i]) {
            // initialise the related hashmap
            sanity_map.push(HashMap::new());
            // add this metadata to index map
            idx_map.insert(metadata_context.local[i], sanity_map.len() - 1);
        } else {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "Policy-specific metadata spec is already in use:\n{:#?}",
                    metadata_context.local[i]
                ),
            ));
        }
    }

    drop(idx_map);
    drop(sanity_map);

    verify_local_specs()
}

#[cfg(feature = "extreme_assertions")]
pub fn bzero(metadata_spec: SideMetadataSpec, start: Address, size: usize) -> Result<()> {
    match spec_to_index(&metadata_spec) {
        Some(idx) => {
            let sanity_map = &mut SANITY_MAP.write().unwrap()[idx];
            // remove add entries where the key (data_addr) is in the range (start, start+size)
            sanity_map.retain(|key, _| *key < start || *key >= start + size);
            Ok(())
        }
        None => Err(Error::new(
            ErrorKind::InvalidInput,
            "Invalid Metadata Spec!",
        )),
    }
}

#[cfg(feature = "extreme_assertions")]
pub fn load(metadata_spec: &SideMetadataSpec, data_addr: Address) -> Result<usize> {
    println!("load({}, {})", metadata_spec.offset, data_addr);
    match spec_to_index(metadata_spec) {
        Some(idx) => {
            let sanity_map = &SANITY_MAP.read().unwrap()[idx];
            match sanity_map.get(&data_addr) {
                Some(val) => Ok(*val),
                None => Err(Error::new(ErrorKind::InvalidInput, "Invalid Data Address!")),
            }
        }
        None => Err(Error::new(
            ErrorKind::InvalidInput,
            "Invalid Metadata Spec!",
        )),
    }
}

#[cfg(feature = "extreme_assertions")]
pub fn store(metadata_spec: SideMetadataSpec, data_addr: Address, metadata: usize) -> Result<()> {
    println!(
        "store({}, {}, {})",
        metadata_spec.offset, data_addr, metadata
    );
    match spec_to_index(&metadata_spec) {
        Some(idx) => {
            let sanity_map = &mut SANITY_MAP.write().unwrap()[idx];
            let content = sanity_map.entry(data_addr).or_insert(0);
            *content = metadata;
            Ok(())
        }
        None => Err(Error::new(
            ErrorKind::InvalidInput,
            "Invalid Metadata Spec!",
        )),
    }
}

#[cfg(feature = "extreme_assertions")]
fn do_math(
    metadata_spec: SideMetadataSpec,
    data_addr: Address,
    val: usize,
    math_op: MathOp,
) -> Result<usize> {
    match spec_to_index(&metadata_spec) {
        Some(idx) => {
            let sanity_map = &mut SANITY_MAP.write().unwrap()[idx];
            let cur_val = sanity_map.entry(data_addr).or_insert(0);
            let old_val = *cur_val;
            match math_op {
                MathOp::Add => *cur_val += val,
                MathOp::Sub => *cur_val -= val,
            }
            Ok(old_val)
        }
        None => Err(Error::new(
            ErrorKind::InvalidInput,
            "Invalid Metadata Spec!",
        )),
    }
}

#[cfg(feature = "extreme_assertions")]
pub fn add(metadata_spec: SideMetadataSpec, data_addr: Address, val: usize) -> Result<usize> {
    do_math(metadata_spec, data_addr, val, MathOp::Add)
}

#[cfg(feature = "extreme_assertions")]
pub fn sub(metadata_spec: SideMetadataSpec, data_addr: Address, val: usize) -> Result<usize> {
    do_math(metadata_spec, data_addr, val, MathOp::Sub)
}

#[test]
fn test_side_metadata_sanity_verify_global_specs_total_size() {
    let spec_1 = SideMetadataSpec {
        scope: super::SideMetadataScope::Global,
        offset: 0,
        log_min_obj_size: 0,
        log_num_of_bits: 0,
    };
    let spec_2 = SideMetadataSpec {
        scope: super::SideMetadataScope::Global,
        offset: super::metadata_address_range_size(spec_1),
        log_min_obj_size: 0,
        log_num_of_bits: 0,
    };

    assert!(verify_global_specs_total_size(&[spec_1]).is_ok());
    #[cfg(target_pointer_width = "64")]
    assert!(verify_global_specs_total_size(&[spec_1, spec_2]).is_ok());
    #[cfg(target_pointer_width = "32")]
    assert!(verify_global_specs_total_size(&[spec_1, spec_2]).is_err());

    let spec_2 = SideMetadataSpec {
        scope: super::SideMetadataScope::Global,
        offset: super::metadata_address_range_size(spec_1),
        log_min_obj_size: 1,
        log_num_of_bits: 3,
    };

    assert!(verify_global_specs_total_size(&[spec_1, spec_2]).is_err());

    let spec_1 = SideMetadataSpec {
        scope: super::SideMetadataScope::Global,
        offset: 0,
        #[cfg(target_pointer_width = "64")]
        log_min_obj_size: 0,
        #[cfg(target_pointer_width = "32")]
        log_min_obj_size: 2,
        log_num_of_bits: 1,
    };
    let spec_2 = SideMetadataSpec {
        scope: super::SideMetadataScope::Global,
        offset: super::metadata_address_range_size(spec_1),
        #[cfg(target_pointer_width = "64")]
        log_min_obj_size: 2,
        #[cfg(target_pointer_width = "32")]
        log_min_obj_size: 4,
        log_num_of_bits: 3,
    };

    assert!(verify_global_specs_total_size(&[spec_1, spec_2]).is_ok());
    assert!(verify_global_specs_total_size(&[spec_1, spec_2, spec_1]).is_err());
}

#[test]
fn test_side_metadata_sanity_verify_no_overlap_contiguous() {
    let spec_1 = SideMetadataSpec {
        scope: super::SideMetadataScope::Global,
        offset: 0,
        log_min_obj_size: 0,
        log_num_of_bits: 0,
    };
    let spec_2 = SideMetadataSpec {
        scope: super::SideMetadataScope::Global,
        offset: super::metadata_address_range_size(spec_1),
        log_min_obj_size: 0,
        log_num_of_bits: 0,
    };

    assert!(verify_no_overlap_contiguous(&spec_1, &spec_1).is_err());
    assert!(verify_no_overlap_contiguous(&spec_1, &spec_2).is_ok());

    let spec_1 = SideMetadataSpec {
        scope: super::SideMetadataScope::Global,
        offset: 1,
        log_min_obj_size: 0,
        log_num_of_bits: 0,
    };

    assert!(verify_no_overlap_contiguous(&spec_1, &spec_2).is_err());

    let spec_1 = SideMetadataSpec {
        scope: super::SideMetadataScope::Global,
        offset: 0,
        log_min_obj_size: 0,
        log_num_of_bits: 0,
    };
    let spec_2 = SideMetadataSpec {
        scope: super::SideMetadataScope::Global,
        offset: super::metadata_address_range_size(spec_1) - 1,
        log_min_obj_size: 0,
        log_num_of_bits: 0,
    };

    assert!(verify_no_overlap_contiguous(&spec_1, &spec_2).is_err());
}

#[cfg(target_pointer_width = "32")]
#[test]
fn test_side_metadata_sanity_verify_no_overlap_chunked() {
    let spec_1 = SideMetadataSpec {
        scope: super::SideMetadataScope::Global,
        offset: 0,
        log_min_obj_size: 0,
        log_num_of_bits: 0,
    };
    let spec_2 = SideMetadataSpec {
        scope: super::SideMetadataScope::Global,
        offset: super::meta_bytes_per_chunk(spec_1.log_min_obj_size, spec_1.log_num_of_bits),
        log_min_obj_size: 0,
        log_num_of_bits: 0,
    };

    assert!(verify_no_overlap_chunked(&spec_1, &spec_1).is_err());
    assert!(verify_no_overlap_chunked(&spec_1, &spec_2).is_ok());

    let spec_1 = SideMetadataSpec {
        scope: super::SideMetadataScope::Global,
        offset: 1,
        log_min_obj_size: 0,
        log_num_of_bits: 0,
    };

    assert!(verify_no_overlap_chunked(&spec_1, &spec_2).is_err());

    let spec_1 = SideMetadataSpec {
        scope: super::SideMetadataScope::Global,
        offset: 0,
        log_min_obj_size: 0,
        log_num_of_bits: 0,
    };
    let spec_2 = SideMetadataSpec {
        scope: super::SideMetadataScope::Global,
        offset: super::meta_bytes_per_chunk(spec_1.log_min_obj_size, spec_1.log_num_of_bits) - 1,
        log_min_obj_size: 0,
        log_num_of_bits: 0,
    };

    assert!(verify_no_overlap_chunked(&spec_1, &spec_2).is_err());
}

#[cfg(target_pointer_width = "32")]
#[test]
fn test_side_metadata_sanity_verify_local_specs_size() {
    let spec_1 = SideMetadataSpec {
        scope: super::SideMetadataScope::PolicySpecific,
        offset: 0,
        log_min_obj_size: 0,
        log_num_of_bits: 0,
    };

    assert!(verify_local_specs_size(&[spec_1]).is_ok());
    assert!(verify_local_specs_size(&[spec_1, spec_1]).is_err());
    assert!(verify_local_specs_size(&[spec_1, spec_1, spec_1, spec_1, spec_1]).is_err());
}
