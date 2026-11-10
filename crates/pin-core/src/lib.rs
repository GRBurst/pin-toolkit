#[cfg(feature = "gitlab")]
pub mod gitlab;
pub mod oci;
pub mod report;
pub mod ver;

// C1 alias: the moved registry.rs says `crate::image_ref`; removed in C3.
#[cfg(feature = "oci")]
pub(crate) use oci::image_ref;
