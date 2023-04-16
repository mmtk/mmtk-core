window.SIDEBAR_ITEMS = {"constant":[["GLOBAL_SIDE_METADATA_BASE_ADDRESS",""],["GLOBAL_SIDE_METADATA_VM_BASE_ADDRESS","The base address for the global side metadata space available to VM bindings, to be used for the per-object metadata. VM bindings must use this to avoid overlap with core internal global side metadata."],["GLOBAL_SIDE_METADATA_VM_BASE_OFFSET","The base offset for the global side metadata available to VM bindings."],["LOCAL_SIDE_METADATA_VM_BASE_OFFSET","The base address for the local side metadata space available to VM bindings, to be used for the per-object metadata. VM bindings must use this to avoid overlap with core internal local side metadata."],["LOG_MAX_GLOBAL_SIDE_METADATA_SIZE",""],["VO_BIT_SIDE_METADATA_ADDR",""]],"fn":[["metadata_address_range_size",""]],"struct":[["MetadataByteArrayRef","A byte array in side-metadata"],["SideMetadataContext","This struct stores all the side metadata specs for a policy. Generally a policy needs to know its own side metadata spec as well as the plan’s specs."],["SideMetadataSanity","This struct includes a hashmap to store the metadata specs information for policy-specific and global metadata for each plan. It uses policy name (or GLOBAL_META_NAME for globals) as the key and keeps a vector of specs as the value. Each plan needs its exclusive instance of this struct to use side metadata specification and content sanity checker."],["SideMetadataSpec","This struct stores the specification of a side metadata bit-set. It is used as an input to the (inline) functions provided by the side metadata module."]],"union":[["SideMetadataOffset","A union of Address or relative offset (usize) used to store offset for a side metadata spec. If a spec is contiguous side metadata, it uses address. Othrewise it uses usize."]]};