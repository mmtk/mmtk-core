#[macro_use]
pub mod layout;
pub mod monotonepageresource;
pub mod pageresource;
mod vmrequest;

pub use self::monotonepageresource::MonotonePageResource;
pub use self::pageresource::PageResource;
pub use self::vmrequest::VMRequest;