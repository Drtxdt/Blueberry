use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use shellsense::{config, engine::CommandIndex, host, probe};
use std::{io::Write, path::PathBuf};

#[derive(Parser)]
#[command(
    version,
    about = "Native PowerShell completion. Run without a subcommand to start pwsh."
)]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Start one PowerShell session inside the native completion host.
    Run {
        #[arg(long, default_value = "pwsh.exe")]
        shell: PathBuf,
        #[arg(long)]
        no_profile: bool,
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// Write numeric performance events to a new JSONL file (no input text).
        #[arg(long)]
        trace: Option<PathBuf>,
    },
    /// Complete an input line without starting PowerShell (cursor is a UTF-8 byte offset).
    Complete {
        #[arg(long)]
        line: String,
        #[arg(long)]
        cursor: Option<usize>,
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Create or validate a TOML theme and behavior file.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Print the current theme using sample candidates.
    Theme,
    /// Print a Windows Terminal profile to add to settings.json.
    TerminalProfile,
    /// Print runtime and configuration diagnostics.
    Doctor,
    /// Exercise the real PowerShell adapter over ConPTY and report timings.
    Probe {
        #[arg(long, default_value = "pwsh.exe")]
        shell: PathBuf,
        #[arg(long,default_value_t=3,value_parser=clap::value_parser!(u16).range(1..101))]
        iterations: u16,
        #[arg(long)]
        with_profile: bool,
        #[arg(long, conflicts_with = "with_profile")]
        host: bool,
        #[arg(long, requires = "host")]
        no_descriptions: bool,
        /// Measure a frozen host release with this probe harness.
        #[arg(long, requires = "host")]
        host_executable: Option<PathBuf>,
        /// Diagnostic run only: enable host traces in this directory.
        #[arg(long, requires = "host")]
        trace_dir: Option<PathBuf>,
        /// Compare a frozen adapter script using the current measurement harness.
        #[arg(long, conflicts_with = "host")]
        adapter_script: Option<PathBuf>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
}
#[derive(Subcommand)]
enum ConfigCommand {
    Init {
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Check,
}

fn execute() -> Result<u32> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Run {
        shell: "pwsh.exe".into(),
        no_profile: false,
        data_dir: None,
        trace: None,
    }) {
        Command::Run {
            shell,
            no_profile,
            data_dir,
            trace,
        } => host::run(host::RunOptions {
            shell,
            no_profile,
            config_path: cli.config,
            data_dir: data_dir.unwrap_or_else(config::cache_dir),
            trace_path: trace,
        }),
        Command::Complete {
            line,
            cursor,
            cwd,
            json,
        } => {
            let settings = config::load(cli.config.as_deref())?;
            let cwd = cwd.unwrap_or(std::env::current_dir()?);
            let result = CommandIndex::discover().complete_with_descriptions(
                &line,
                cursor.unwrap_or(line.len()),
                &cwd,
                settings.completion.max_results,
                &settings.descriptions,
            );
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                for candidate in result.candidates {
                    println!("{}\t{}", candidate.label, candidate.description);
                }
            }
            Ok(0)
        }
        Command::Config {
            command: ConfigCommand::Init { output },
        } => {
            let path = output.or(cli.config).unwrap_or_else(config::default_path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .with_context(|| format!("Will not overwrite {}", path.display()))?;
            file.write_all(config::example().as_bytes())?;
            println!("{}", path.display());
            Ok(0)
        }
        Command::Config {
            command: ConfigCommand::Check,
        } => {
            let settings = config::load(cli.config.as_deref())?;
            settings.validate()?;
            println!(
                "Configuration valid: {}",
                cli.config.unwrap_or_else(config::default_path).display()
            );
            Ok(0)
        }
        Command::Theme => {
            let settings = config::load(cli.config.as_deref())?;
            let result = CommandIndex::discover().complete_with_descriptions(
                "git ",
                4,
                &std::env::current_dir()?,
                20,
                &settings.descriptions,
            );
            let frame = shellsense::menu::render(&result.candidates, 0, "", 100, &settings);
            for line in frame.lines {
                println!("{line}");
            }
            Ok(0)
        }
        Command::TerminalProfile => {
            let executable = std::env::current_exe()?;
            let profile = serde_json::json!({
                "name":"ShellSense PowerShell",
                "commandline":format!("\"{}\" run",executable.display()),
                "startingDirectory":"%USERPROFILE%",
                "guid":"{cda29eb9-0b16-4c08-bc90-75d9275a8ae9}",
                "hidden":false
            });
            println!("{}", serde_json::to_string_pretty(&profile)?);
            Ok(0)
        }
        Command::Doctor => {
            let settings = config::load(cli.config.as_deref())?;
            println!(
                "ShellSense {}\nPlatform: {} / {}\nConfig: {}\nCache: {}\nMax candidates: {}\nRuntime: Rust + existing PowerShell/PSReadLine; no Node runtime",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH,
                cli.config.unwrap_or_else(config::default_path).display(),
                config::cache_dir().display(),
                settings.completion.max_results
            );
            Ok(0)
        }
        Command::Probe {
            shell,
            iterations,
            with_profile,
            host,
            no_descriptions,
            host_executable,
            trace_dir,
            adapter_script,
            output,
        } => {
            let report = if host {
                shellsense::metrics::host_probe_traced(
                    &host_executable.unwrap_or(std::env::current_exe()?),
                    &shell,
                    iterations,
                    !no_descriptions,
                    trace_dir.as_deref(),
                )?
            } else {
                probe::run_with_adapter(
                    &shell,
                    iterations,
                    !with_profile,
                    adapter_script.as_deref(),
                )?
            };
            let text = serde_json::to_string_pretty(&report)?;
            if let Some(path) = output {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, &text)?;
            }
            println!("{text}");
            Ok(0)
        }
    }
}
fn main() -> std::process::ExitCode {
    match execute() {
        Ok(code) => std::process::ExitCode::from(code.min(255) as u8),
        Err(error) => {
            eprintln!("shellsense: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
