#![forbid(unsafe_code)]
//! Readings and where they come from.
//!
//! The vocabulary says what a key means; a provider says what this machine can
//! read; the registry turns many displays' asks into one poll. Nothing here
//! formats a number or converts a unit: a reading stays as measured until it is
//! presented.

pub mod clock;
pub mod provider;
pub mod registry;
pub mod sample;
pub mod system;
pub mod units;
pub mod vocabulary;
