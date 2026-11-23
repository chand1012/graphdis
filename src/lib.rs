#![no_std]

//! Graphdis configuration and embedded application services.

extern crate alloc;

pub mod config;

#[cfg(feature = "http")]
pub mod sources;

#[cfg(any(feature = "firmware", test))]
mod retry;

#[cfg(feature = "firmware")]
pub mod timing;

#[cfg(feature = "firmware")]
pub mod network;

#[cfg(feature = "display")]
pub mod display;

#[cfg(feature = "firmware")]
pub mod app;
#[cfg(feature = "firmware")]
pub mod board;
#[cfg(feature = "metrics")]
pub mod metrics;
