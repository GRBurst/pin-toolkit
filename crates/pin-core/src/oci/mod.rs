#[cfg(feature = "oci")]
mod decide;
pub mod image_ref;
// D38 (review M3): the raw registry client is reachable from outside the
// crate only under `test-support` (dev-dependencies); plugins resolve
// through `Ctx`.
#[cfg(all(feature = "oci", feature = "test-support"))]
#[doc(hidden)]
pub mod registry;
#[cfg(all(feature = "oci", not(feature = "test-support")))]
pub(crate) mod registry;
pub mod splice;

#[cfg(feature = "oci")]
pub use decide::{ImageDecision, decide_image, pin_inline_image};
pub use image_ref::ImageRef;
#[cfg(feature = "oci")]
pub use registry::RegError;
#[cfg(all(feature = "oci", feature = "test-support"))]
#[doc(hidden)]
pub use registry::RegistryClient;
pub use splice::{splice_update, splice_value};
