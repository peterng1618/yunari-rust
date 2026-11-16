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
pub mod mcp;
pub mod pages;
pub mod pairing;
pub mod panels;
pub mod panels_routes;
pub mod server;
pub mod store;
pub mod stream;
pub mod themes_routes;
pub mod tls;

pub use config::HostConfig;
pub use identity::HostIdentity;
