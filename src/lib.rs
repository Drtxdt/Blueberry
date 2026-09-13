pub mod beta_metrics;
pub mod cache_writer;
pub mod completion;
pub mod config;
pub mod engine;
pub mod host;
pub mod input;
pub mod menu;
pub mod metrics;
pub mod model;
pub mod overlay;
pub mod paths;
pub mod pipe;
pub mod probe;
pub mod protocol;
pub mod providers;
pub mod pty;
pub mod ranking;
pub mod reload;
pub mod sources;
pub mod spec_catalog;
pub mod specs;
pub mod status;
pub mod trace;

#[cfg(windows)]
pub(crate) mod vt_input;

#[cfg(windows)]
pub(crate) mod windows_input;

pub mod knowledge;

pub mod startup;
