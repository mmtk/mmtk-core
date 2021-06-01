use super::MetadataContext;

pub(crate) type HeaderMetadataContext = MetadataContext;

pub(crate) struct HeaderMetadata {
    context: HeaderMetadataContext,
}

impl HeaderMetadata {
    pub(crate) fn new(context: HeaderMetadataContext) -> HeaderMetadata {
        Self { context }
    }

    pub(crate) fn get_context(&self) -> &HeaderMetadataContext {
        &self.context
    }
}
