#![forbid(unsafe_code)]
//! The display host: TLS, pairing, samples and displays.

pub mod config;
pub mod displays;
pub mod http;
pub mod identity;
pub mod pairing;
pub mod pages;
pub mod lock;
pub mod server;
pub mod tls;

pub use config::HostConfig;
pub use identity::HostIdentity;
