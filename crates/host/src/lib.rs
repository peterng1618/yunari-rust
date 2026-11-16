#![forbid(unsafe_code)]
//! The display host: TLS, pairing, samples and displays.

pub mod assignment;
pub mod autostart;
pub mod config;
pub mod credentials;
pub mod displays;
pub mod frame;
pub mod hosting;
pub mod http;
pub mod identity;
pub mod lock;
pub mod mcp;
pub mod origins;
pub mod packs_routes;
pub mod pages;
pub mod pairing;
pub mod panels;
pub mod panels_routes;
pub mod server;
pub mod store;
pub mod store_install;
pub mod store_routes;
pub mod stream;
pub mod submit;
pub mod themes_routes;
pub mod thumbnail;
pub mod tls;

pub use config::HostConfig;
pub use identity::HostIdentity;
