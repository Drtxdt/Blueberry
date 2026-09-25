pub mod beta_metrics;
pub mod cache_writer;
pub mod completion;
pub mod config;
pub mod engine;
pub mod host;
pub mod hub;
pub mod input;
pub mod menu;
pub mod metrics;
pub mod model;
pub mod overlay;
pub mod packs;
pub mod paths;
pub mod pipe;
pub mod probe;
pub mod protocol;
pub mod providers;
pub mod pty;
pub mod ranking;
pub mod reload;
pub mod settings;
pub mod setup;
pub mod sources;
pub mod spec_catalog;
pub mod specs;
pub mod status;
pub mod terminal_ui;
pub mod tool_registry;
pub mod tools_ui;
pub mod trace;

#[cfg(windows)]
pub(crate) mod vt_input;

#[cfg(windows)]
pub(crate) mod windows_input;

pub mod knowledge;

#[cfg(windows)]
pub mod latency_layers;

pub mod startup;
