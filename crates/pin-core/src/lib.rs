pub mod edit;
pub mod git;
// D38 (review M3): the raw GitLab client only under `test-support`.
#[cfg(all(feature = "gitlab", feature = "test-support"))]
#[doc(hidden)]
pub mod gitlab;
#[cfg(all(feature = "gitlab", not(feature = "test-support")))]
#[allow(dead_code)] // items only tests (test-support) use
pub(crate) mod gitlab;
pub mod oci;
pub mod plugin;
pub mod report;
pub mod run;
pub(crate) mod secret;
pub mod text;
pub mod ver;
#[cfg(feature = "yaml")]
pub mod yaml;
