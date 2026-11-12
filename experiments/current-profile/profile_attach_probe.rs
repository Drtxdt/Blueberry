//! Disposable startup experiment. Never installed or used by the managed Profile.
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use blueberry::host::{PsReadLineVersion, RunOptions, Transport, direct};
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 2 && args[0] == "doctor" && args[1] == "--json" {
        println!(
            "{}",
            serde_json::json!({"build":{
                "commit":env!("BLUEBERRY_BUILD_COMMIT"),"profile":env!("BLUEBERRY_BUILD_PROFILE"),
                "dirty":env!("BLUEBERRY_BUILD_DIRTY")=="true","experimental_profile_service":true,
                "private_editors":blueberry::editor::build_identity(),"conpty":blueberry::conpty::build_identity()
            }})
        );
        return Ok(());
    }
    ensure!(
        args.len() == 4,
        "expected shell PID, shell path, data directory and editor version"
    );
    let pid = args[0].to_str().context("invalid PID")?.parse()?;
    let editor = match args[3].to_str() {
        Some("2.0.0") => PsReadLineVersion::V200,
        Some("2.4.5") => PsReadLineVersion::V245,
        _ => anyhow::bail!("unsupported editor"),
    };
    direct::probe_current_editor(
        RunOptions {
            shell: (&args[1]).into(),
            data_dir: (&args[2]).into(),
            no_profile: false,
            transport: Transport::Pipe,
            config_path: None,
            psreadline_version: editor,
            trace: blueberry::trace::Trace::open(None)?,
        },
        pid,
    )?;
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows only");
    std::process::exit(1);
}
