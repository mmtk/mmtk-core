use crate::policy::sft::SFT;
use crate::policy::space::Space;
use crate::util::linear_scan::RegionIterator;
#[cfg(feature = "vo_bit")]
use crate::util::ObjectReference;
use crate::util::{linear_scan::Region, Address};

/// SpaceInspector allows users to inspect the heap in a hierarchical structure.
pub trait SpaceInspector: SFT
where
    Self: 'static,
{
    /// The name of this space, given by the plan.
    fn space_name(&self) -> &str {
        SFT::name(self)
    }
    /// The name of this policy.
    fn policy_name(&self) -> &str {
        std::any::type_name::<Self>()
    }
    fn used_pages(&self) -> usize;
    /// List the top-level regions used by this space. This is usually [`crate::util::heap::chunk_map::Chunk`] for most spaces.
    /// If there is no region used by the space at the moment, it returns an empty Vector.
    fn list_top_regions(&self) -> Vec<Box<dyn RegionInspector>>;
    /// List sub regions of the given parent region if the space organises its heap in a heirarchical way.
    /// The parent region could be the results from [`SpaceInspector::list_top_regions`] or the results from [`SpaceInspector::list_sub_regions`].
    /// If there is no sub regions for the given region, it returns an empty Vector.
    fn list_sub_regions(
        &self,
        parent_region: &dyn RegionInspector,
    ) -> Vec<Box<dyn RegionInspector>>;
}

/// For the given region inspector, if it matches the PARENT region, return the sub regions of the CHILD region type.
/// Otherwise, return None.
pub(crate) fn list_sub_regions<PARENT: Region + 'static, CHILD: Region + 'static>(
    region: &dyn RegionInspector,
) -> Option<Vec<Box<dyn RegionInspector>>> {
    if region.region_type() == std::any::type_name::<PARENT>() {
        let start_child_region = CHILD::from_aligned_address(region.start());
        let end_child_region = CHILD::from_aligned_address(region.start() + region.size());
        Some(
            RegionIterator::<CHILD>::new(start_child_region, end_child_region)
                .map(|r| Box::new(r) as Box<dyn RegionInspector>)
                .collect(),
        )
    } else {
        None
    }
}

/// Convert an iterator of pairs of (region start, region size) into a vector of region inspector.
pub(crate) fn into_regions<R: Region + 'static>(
    regions: &mut dyn Iterator<Item = (Address, usize)>,
) -> Vec<Box<dyn RegionInspector>> {
    regions
        .flat_map(|(start, size)| {
            let mut current = start;
            let end = start + size;
            std::iter::from_fn(move || {
                if current >= end {
                    return None;
                }
                let region = R::from_aligned_address(current);
                current += R::BYTES;
                Some(Box::new(region) as Box<dyn RegionInspector>)
            })
        })
        .collect()
}

/// RegionInspector allows users to inspect a region of the heap.
pub trait RegionInspector {
    /// The type of this region.
    fn region_type(&self) -> &str;
    /// The start address of this region.
    fn start(&self) -> Address;
    /// The byte size of this region.
    fn size(&self) -> usize;
    #[cfg(feature = "vo_bit")]
    /// List all objects in this region. This is only available when `vo_bit` feature is enabled.
    fn list_objects(&self) -> Vec<ObjectReference> {
        let mut objects = vec![];
        crate::util::metadata::side_metadata::spec_defs::VO_BIT.scan_non_zero_values::<u8>(
            self.start(),
            self.start() + self.size(),
            &mut |address| {
                use crate::util::metadata::vo_bit;
                let object = vo_bit::get_object_ref_for_vo_addr(address);
                objects.push(object);
            },
        );
        objects
    }
}

impl<R: Region + 'static> RegionInspector for R {
    fn region_type(&self) -> &str {
        std::any::type_name::<R>()
    }

    fn start(&self) -> Address {
        Region::start(self)
    }

    fn size(&self) -> usize {
        Self::BYTES
    }
}

use crate::vm::VMBinding;
/// SpaceAsRegion is a special RegionInspector. Some spaces do not organize its space memory with regions.
/// For those spaces, they simply return this type as the top-level region inspector so users can inspect
/// such spaces in the same way as other spaces that use regions.
pub(crate) struct SpaceAsRegion<VM: VMBinding> {
    space: &'static dyn Space<VM>,
}

impl<VM: VMBinding> SpaceAsRegion<VM> {
    pub fn new(space: &'static dyn Space<VM>) -> Self {
        Self { space }
    }
}

impl<VM: VMBinding> RegionInspector for SpaceAsRegion<VM> {
    fn region_type(&self) -> &str {
        std::any::type_name::<SpaceAsRegion<VM>>()
    }

    fn start(&self) -> Address {
        Address::ZERO
    }

    fn size(&self) -> usize {
        0
    }

    #[cfg(feature = "vo_bit")]
    fn list_objects(&self) -> Vec<ObjectReference> {
        let mut res = vec![];
        let mut enumerator =
            crate::util::object_enum::ClosureObjectEnumerator::<_, VM>::new(|object| {
                res.push(object);
            });
        self.space.enumerate_objects(&mut enumerator);
        res
    }
}
