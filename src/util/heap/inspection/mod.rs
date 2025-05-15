use crate::policy::sft::SFT;
use crate::util::linear_scan::RegionIterator;
use crate::util::{linear_scan::Region, Address, ObjectReference};
use crate::util::metadata::side_metadata::spec_defs::VO_BIT;
use crate::util::heap::chunk_map::Chunk;

pub trait SpaceInspector {
    fn name(&self) -> &str;
    fn list_regions(&self, parent_region: Option<&dyn RegionInspector>) -> Vec<Box<dyn RegionInspector>>;
}

pub(crate) fn list_child_regions<PARENT: Region, CHILD: Region + 'static>(region: &dyn RegionInspector) -> Option<Vec<Box<dyn RegionInspector>>> {
    if region.region_type() == std::any::type_name::<PARENT>() {
        let start_child_region = CHILD::from_aligned_address(region.start());
        let end_child_region = CHILD::from_aligned_address(region.start() + region.size());
        Some(RegionIterator::<CHILD>::new(start_child_region, end_child_region)
            .map(|r| Box::new(r) as Box<dyn RegionInspector>)
            .collect())
    } else {
        None
    }
}

pub trait RegionInspector {
    fn region_type(&self) -> &str;
    fn start(&self) -> Address;
    fn size(&self) -> usize;
    #[cfg(feature = "vo_bit")]
    fn list_objects(&self) -> Vec<ObjectReference> {
        let mut objects = vec![];
        VO_BIT.scan_non_zero_values::<u8>(self.start(), self.start() + self.size(), &mut |address| {
            use crate::util::metadata::vo_bit;
            let object = vo_bit::get_object_ref_for_vo_addr(address);
            objects.push(object);
        });
        objects
    }
}

impl<R: Region> RegionInspector for R {
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
