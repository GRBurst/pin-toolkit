#[cfg(feature = "oci")]
mod decide;
pub mod image_ref;
#[cfg(feature = "oci")]
pub mod registry;
pub mod splice;

#[cfg(feature = "oci")]
pub use decide::{ImageDecision, decide_image, pin_inline_image};
pub use image_ref::ImageRef;
#[cfg(feature = "oci")]
pub use registry::RegistryClient;
pub use splice::{splice_update, splice_value};
