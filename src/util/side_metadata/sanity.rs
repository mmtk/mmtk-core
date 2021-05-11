use crate::util::Address;
use std::collections::HashMap;
use std::io::{Error, ErrorKind, Result};
use std::sync::RwLock;

use super::{SideMetadataContext, SideMetadataSpec};

enum MathOp {
    Add,
    Sub,
}

lazy_static! {
    static ref SANITY_MAP: RwLock<Vec<HashMap<Address, usize>>> = RwLock::new(vec![]);
    static ref SPEC_TO_IDX_MAP: RwLock<HashMap<SideMetadataSpec, usize>> =
        RwLock::new(HashMap::new());
}

fn spec_to_index(metadata_spec: &SideMetadataSpec) -> Option<usize> {
    let spec_to_index_map = SPEC_TO_IDX_MAP.read().unwrap();
    match spec_to_index_map.get(metadata_spec) {
        Some(idx) => Some(*idx),
        None => None,
    }
}

pub fn add_metadata_context(metadata_context: &SideMetadataContext) {
    // assert not initialised before
    let mut sanity_map = SANITY_MAP.write().unwrap();
    let mut idx_map = SPEC_TO_IDX_MAP.write().unwrap();

    let global_count = metadata_context.global.len();
    let local_count = metadata_context.local.len();

    println!("add_metadata_context.g({}).l({})", global_count, local_count);

    let cur_total_count = sanity_map.len();
    let first_call = cur_total_count == 0;
    // sanity_map.reserve(global_count + local_count);

    for i in 0..global_count {
        if first_call {
            // initialise the related hashmap
            sanity_map.push(HashMap::new());
            // add this metadata to index map
            idx_map.insert(metadata_context.global[i], i);
        } else {
            assert!(
                idx_map.contains_key(&metadata_context.global[i]),
                "Global metadata must not change between policies"
            );
        }
    }

    for i in 0..local_count {
        if !idx_map.contains_key(&metadata_context.local[i]) {
            // initialise the related hashmap
            sanity_map.push(HashMap::new());
            // add this metadata to index map
            idx_map.insert(metadata_context.local[i], sanity_map.len() - 1);
        }
    }
}

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

pub fn store(metadata_spec: SideMetadataSpec, data_addr: Address, metadata: usize) -> Result<()> {
    println!("store({}, {}, {})", metadata_spec.offset, data_addr, metadata);
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

fn do_math(
    metadata_spec: SideMetadataSpec,
    data_addr: Address,
    val: usize,
    math_op: MathOp,
) -> Result<usize> {
    match spec_to_index(&metadata_spec) {
        Some(idx) => {
            let sanity_map = &mut SANITY_MAP.write().unwrap()[idx];
            match sanity_map.get_mut(&data_addr) {
                Some(cur_val) => {
                    let old_val = *cur_val;
                    match math_op {
                        MathOp::Add => *cur_val += val,
                        MathOp::Sub => *cur_val -= val,
                    }
                    Ok(old_val)
                }
                None => Err(Error::new(ErrorKind::InvalidInput, "Invalid Data Address!")),
            }
        }
        None => Err(Error::new(
            ErrorKind::InvalidInput,
            "Invalid Metadata Spec!",
        )),
    }
}

pub fn add(metadata_spec: SideMetadataSpec, data_addr: Address, val: usize) -> Result<usize> {
    do_math(metadata_spec, data_addr, val, MathOp::Add)
}

pub fn sub(metadata_spec: SideMetadataSpec, data_addr: Address, val: usize) -> Result<usize> {
    do_math(metadata_spec, data_addr, val, MathOp::Sub)
}
