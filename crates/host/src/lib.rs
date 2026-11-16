#![forbid(unsafe_code)]
//! The display host: TLS, pairing, samples and displays.

pub mod config;
pub mod displays;
pub mod identity;
pub mod lock;

pub use config::HostConfig;
pub use identity::HostIdentity;
