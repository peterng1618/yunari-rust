#![recursion_limit = "256"]
#![forbid(unsafe_code)]
//! The display host: TLS, pairing, samples and displays.

pub mod assets;
pub mod assignment;
pub mod autostart;
pub mod config;
pub mod credentials;
pub mod device_routes;
pub mod displays;
pub mod drafts;
pub mod frame;
pub mod hosting;
pub mod http;
pub mod identity;
pub mod lock;
pub mod mcp;
pub mod mcp_compose;
pub mod mcp_install;
pub mod monitors;
pub mod origins;
pub mod packs_routes;
pub mod pages;
pub mod pairing;
pub mod panel_video;
pub mod panels;
pub mod panels_routes;
pub mod path_presets;
pub mod server;
pub mod signalrgb;
pub mod store;
pub mod store_install;
pub mod store_routes;
pub mod stream;
pub mod submit;
pub mod themes_routes;
pub mod thumbnail;
pub mod tls;
pub mod wallpaper_engine;

pub use config::HostConfig;
pub use identity::HostIdentity;
