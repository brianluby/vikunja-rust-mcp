//! Standalone async client for the Vikunja REST API (`/api/v1`).

pub mod client;
pub mod models;
pub mod pagination;

pub use client::VikunjaClient;
