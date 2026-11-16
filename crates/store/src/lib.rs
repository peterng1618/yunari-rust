#![deny(unsafe_code)]
//! The store account and its Worker client.
//!
//! Two owners live here: [`session`] is the token this host holds after a
//! sign-in, written under the OS protector rather than in plain text; [`client`]
//! is the author's calls to the Worker, each one carrying that token. The
//! token never leaves this crate except into an `Authorization` header.

pub mod client;
pub mod secret;
pub mod session;

pub use client::{Client, ClientError, CreateInput, Reply, Request, send};
pub use session::{Session, SessionStore};
