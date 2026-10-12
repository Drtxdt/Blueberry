//! Isolated accessibility probe. Reads only the helper's own console window.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Parser)]
struct Options {
    #[arg(long)]
    shell: PathBuf,
    #[arg(long)]
    script: PathBuf,
    #[arg(long)]
    output: PathBuf,
}

fn main() -> Result<()> {
    let options = Options::parse();
    ensure!(!options.output.exists(), "refusing to overwrite evidence");
    let output = std::path::absolute(options.output)?;
    // Windows PowerShell 5.1 rejects Rust's canonical \\?\ script prefix.
    let script = std::path::absolute(options.script)?;
    ensure!(script.is_file(), "missing capability script");
    let mut harness = blueberry::probe::Harness::start(
        &options.shell,
        &[
            "-NoLogo".into(),
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-File".into(),
            script.to_string_lossy().into(),
            "-OutputPath".into(),
            output.to_string_lossy().into(),
        ],
        &std::env::current_dir()?,
        &BTreeMap::from([("BLUEBERRY_ACTIVE".into(), "1".into())]),
        String::new(),
    )?;
    let deadline = Instant::now() + Duration::from_secs(20);
    while !output.exists() && Instant::now() < deadline {
        let _ = harness.pump(Duration::from_millis(25));
    }
    let _ = harness.stop();
    let report = std::fs::read_to_string(&output).with_context(|| {
        format!(
            "capability helper did not produce evidence; own console: {}",
            harness.viewport_contents()
        )
    })?;
    println!("{report}");
    Ok(())
}
