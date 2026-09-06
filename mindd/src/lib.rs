//! MindOS mind: shared protocol, configuration and client code for the
//! `mindd` daemon and the `mind` CLI.

pub mod client;
pub mod config;
pub mod daemon;
pub mod proto;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
