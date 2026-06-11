//! MCP server exposing the Vikunja REST API as Model Context Protocol tools
//! and resources.
//!
//! The crate is split into three layers:
//! - [`config`]: CLI/environment configuration and validation.
//! - [`vikunja`]: a standalone async client for the Vikunja REST API.
//! - [`mcp`]: the MCP server, tool and resource definitions built on `rmcp`.

pub mod config;
pub mod dates;
pub mod error;
pub mod mcp;
pub mod metrics;
pub mod sandbox;
pub mod schema;
pub mod vikunja;
