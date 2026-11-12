//! Diagnostic only: use release builds, and do not mix with formal startup runs.
pub use blueberry::{engine, pty};
use std::{ffi::OsString, time::Instant};
#[cfg(windows)]
#[allow(dead_code)]
#[path = "../src/system_commands_cache.rs"]
mod system_commands_cache;
fn main() -> anyhow::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .unwrap_or_else(|| std::env::var_os("PATH").unwrap_or_default());
    let ext = std::env::var_os("PATHEXT").unwrap_or_else(|| OsString::from(".EXE;.CMD;.BAT;.COM"));
    let mut discovery = engine::Discovery::new(&path, &ext);
    let mut batches = Vec::new();
    loop {
        let scan = Instant::now();
        let done = discovery.step();
        let scan_us = scan.elapsed().as_micros();
        let clone = Instant::now();
        let snapshot = discovery.snapshot();
        let clone_us = clone.elapsed().as_micros();
        std::hint::black_box(snapshot);
        batches.push(serde_json::json!({"scan_us":scan_us,"snapshot_us":clone_us,"done":done}));
        if done {
            break;
        }
    }
    #[cfg(windows)]
    let mut inventory_us = Vec::new();
    #[cfg(not(windows))]
    let inventory_us = Vec::<u128>::new();
    #[cfg(windows)]
    for _ in 0..5 {
        let started = Instant::now();
        std::hint::black_box(system_commands_cache::identity(
            &pty::default_shell(),
            include_str!("../shell/system-commands.ps1"),
        )?);
        inventory_us.push(started.elapsed().as_micros());
    }
    println!(
        "{}",
        serde_json::json!({"diagnostic_only":true,"batches":batches,"module_inventory_us":inventory_us})
    );
    Ok(())
}
