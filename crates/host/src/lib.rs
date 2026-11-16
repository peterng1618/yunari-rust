#![forbid(unsafe_code)]
//! The display host: TLS, pairing, samples and displays.

pub mod assignment;
pub mod config;
pub mod displays;
pub mod frame;
pub mod http;
pub mod identity;
pub mod pairing;
pub mod pages;
pub mod lock;
pub mod server;
pub mod stream;
pub mod tls;

pub use config::HostConfig;
pub use identity::HostIdentity;
