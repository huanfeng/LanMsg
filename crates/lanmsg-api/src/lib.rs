pub mod ipc;
pub mod types;

pub use types::*;

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!("lanmsg_api");
