#![forbid(unsafe_code)]
//! The display host: TLS, pairing, samples and displays.

pub mod assignment;
pub mod autostart;
pub mod config;
pub mod displays;
pub mod frame;
pub mod hosting;
pub mod http;
pub mod identity;
pub mod lock;
pub mod pages;
pub mod pairing;
pub mod server;
pub mod stream;
pub mod themes_routes;
pub mod tls;

pub use config::HostConfig;
pub use identity::HostIdentity;
