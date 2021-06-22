use std::fmt;

use crate::util::metadata::side_metadata::SideMetadataSpec;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct HeaderMetadataSpec {
    pub bit_offset: isize,
    pub num_of_bits: usize,
}

impl fmt::Debug for HeaderMetadataSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!(
            "HeaderMetadataSpec {{ \
            **bit_offset: 0x{:x} \
            **num_of_bits: 0x{:x} \
            }}",
            self.bit_offset, self.num_of_bits
        ))
    }
}

/// This struct stores the specification of a metadata bit-set.
/// It is used as an input to the (inline) functions provided by the side metadata module.
///
/// Each plan or policy which uses a metadata bit-set, needs to create an instance of this struct.
///
/// For performance reasons, objects of this struct should be constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MetadataSpec {
    InHeader(HeaderMetadataSpec),
    OnSide(SideMetadataSpec),
}

impl MetadataSpec {
    pub fn is_on_side(&self) -> bool {
        matches!(self, &MetadataSpec::OnSide(_))
    }
}

/// Given a slice of metadata specifications, returns a vector of the specs which are on side.
///
/// # Arguments:
/// * `specs` is the input slice of on-side and/or in-header metadata specifications.
///
pub(crate) fn extract_side_metadata(specs: &[MetadataSpec]) -> Vec<SideMetadataSpec> {
    let mut side_specs = vec![];
    for spec in specs {
        if let MetadataSpec::OnSide(ss) = *spec {
            side_specs.push(ss);
        }
    }

    side_specs
}
