use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use shellsense::{
    completion, config,
    engine::CommandIndex,
    host,
    model::Completion,
    probe,
    ranking::Learning,
    sources,
    spec_catalog::{Catalog, CatalogLoadError, CheckReport, SpecDiagnostic},
};
use std::{
    collections::BTreeMap,
    env,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

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
    /// Measure six complete-host scenarios with profile and explicit cache states.
    BetaProbe {
        #[arg(long, default_value = "pwsh.exe")]
        shell: PathBuf,
        #[arg(long, default_value_t = 300, value_parser=clap::value_parser!(u16).range(1..10001))]
        samples: u16,
        #[arg(long, value_enum, default_value = "osc")]
        transport: host::Transport,
        #[arg(long)]
        no_descriptions: bool,
        #[arg(long)]
        host_executable: Option<PathBuf>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
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
        /// Explicit transport experiment; OSC remains the default.
        #[arg(long, value_enum, default_value = "osc")]
        transport: host::Transport,
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
        /// Explain the parsed context, selected specification, data sources,
        /// and why each returned candidate was included.
        #[arg(long)]
        explain: bool,
    },
    /// Check and inspect declarative TOML completion specifications.
    Specs {
        #[command(subcommand)]
        command: SpecsCommand,
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
    /// Manage local, hashed candidate selection statistics.
    Learning {
        #[command(subcommand)]
        command: LearningCommand,
    },
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
enum SpecsCommand {
    /// Validate every user specification in the configured directory.
    Check {
        /// Read specifications from this directory instead of the configured path.
        #[arg(long, alias = "dir")]
        directory: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// List effective built-in and user specification contexts.
    List {
        /// Read specifications from this directory instead of the configured path.
        #[arg(long, alias = "dir")]
        directory: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum LearningCommand {
    /// Remove all local selection statistics and start with a new salt.
    Clear,
}
#[derive(Subcommand)]
enum ConfigCommand {
    Init {
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Check,
}

struct CompletionRun {
    completion: Completion,
    request: sources::SourceRequest,
    catalog: Catalog,
    catalog_path: PathBuf,
    source_diagnostics: Vec<String>,
    project_incomplete: bool,
    paths_incomplete: bool,
}

fn validate_cursor(line: &str, cursor: usize) -> Result<()> {
    if cursor > line.len() {
        bail!(
            "cursor must be within the UTF-8 line ({} > {})",
            cursor,
            line.len()
        );
    }
    if !line.is_char_boundary(cursor) {
        bail!("cursor must point to a UTF-8 character boundary");
    }
    Ok(())
}

fn load_catalog(directory: &Path) -> Result<Catalog> {
    Catalog::load_user_dir(directory).map_err(|error| match error {
        CatalogLoadError::Io { path, source } => {
            anyhow!("无法读取用户规格目录 {}: {source}", path.display())
        }
    })
}

fn catalog_directory(settings: &config::Config, config_path: Option<&Path>) -> PathBuf {
    completion::catalog_path(settings, config_path)
}

fn complete_line(
    settings: &config::Config,
    config_path: Option<&Path>,
    line: &str,
    cursor: usize,
    cwd: &Path,
) -> Result<CompletionRun> {
    let catalog_path = catalog_directory(settings, config_path);
    let catalog = load_catalog(&catalog_path)?;
    let index = CommandIndex::discover();
    let environment = Arc::new(env::vars().collect::<BTreeMap<_, _>>());
    let (base, request) = completion::plan(
        &index,
        &catalog,
        line,
        cursor,
        cwd,
        None,
        1,
        environment,
        settings.completion.fuzzy,
        settings.completion.dynamic,
        &settings.descriptions,
    );
    let updates = sources::collect_once(&request);
    let project_incomplete = updates[0].incomplete;
    let paths_incomplete = updates[1].incomplete;
    let source_diagnostics = updates
        .iter()
        .flat_map(|update| update.diagnostics.iter().cloned())
        .collect::<Vec<_>>();
    let usage = settings
        .learning
        .enabled
        .then(|| shellsense::ranking::UsageSnapshot::load(&config::statistics_path()));
    let completion = completion::merge(
        base,
        &request,
        [Some(&updates[0]), Some(&updates[1])],
        usage.as_ref(),
        settings.completion.max_results,
        &settings.descriptions,
    );
    Ok(CompletionRun {
        completion,
        request,
        catalog,
        catalog_path,
        source_diagnostics,
        project_incomplete,
        paths_incomplete,
    })
}

fn diagnostic_json(diagnostic: &SpecDiagnostic) -> serde_json::Value {
    serde_json::json!({
        "source": diagnostic.source,
        "path": diagnostic.path,
        "severity": diagnostic.severity.to_string(),
        "message": diagnostic.message,
        "context": diagnostic.context,
    })
}

fn print_completion_explanation(run: &CompletionRun, json: bool) -> Result<()> {
    let request = &run.request;
    let diagnostics = run
        .catalog
        .diagnostics()
        .iter()
        .map(diagnostic_json)
        .collect::<Vec<_>>();
    if json {
        let candidates = run
            .completion
            .candidates
            .iter()
            .map(|candidate| {
                serde_json::json!({
                    "label": candidate.label,
                    "kind": format!("{:?}", candidate.kind),
                    "source": if candidate.source.is_empty() { "engine" } else { candidate.source.as_str() },
                    "detail": candidate.detail,
                    "reason": if candidate.source.starts_with("builtin") { "built-in specification match" } else if candidate.source.is_empty() { "command or shell snapshot match" } else { "user or dynamic source match" },
                })
            })
            .collect::<Vec<_>>();
        let payload = serde_json::json!({
            "completion": &run.completion,
            "explain": {
                "context": {
                    "command": request.context.command,
                    "arguments": request.context.arguments,
                    "prefix": request.context.prefix,
                    "replace_start": request.context.replace_start,
                    "replace_end": request.context.replace_end,
                    "command_position": request.context.command_position,
                    "suppressed": request.context.suppressed,
                },
                "catalog": {
                    "directory": run.catalog_path,
                    "diagnostics": diagnostics,
                },
                "sources": {
                    "project_requested": request.project,
                    "paths_requested": request.paths,
                    "provider": request.provider,
                    "project_incomplete": run.project_incomplete,
                    "paths_incomplete": run.paths_incomplete,
                    "diagnostics": run.source_diagnostics,
                },
                "matching": {
                    "fuzzy": request.fuzzy,
                    "candidate_count": candidates.len(),
                    "candidates": candidates,
                },
            },
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!("补全解释");
    println!(
        "上下文: command={}  prefix={:?}  替换范围={}..{}  command_position={}  suppressed={}",
        request.context.command,
        request.context.prefix,
        request.context.replace_start,
        request.context.replace_end,
        request.context.command_position,
        request.context.suppressed
    );
    println!(
        "规格目录: {}  规格诊断: {} 条",
        run.catalog_path.display(),
        diagnostics.len()
    );
    println!(
        "数据源: project={} paths={} provider={} incomplete={} (project={}, paths={})",
        request.project,
        request.paths,
        request.provider.as_deref().unwrap_or("无"),
        run.completion.incomplete,
        run.project_incomplete,
        run.paths_incomplete
    );
    for diagnostic in run.catalog.diagnostics() {
        println!(
            "  [{}] {}: {}",
            diagnostic.severity, diagnostic.path, diagnostic.message
        );
    }
    for diagnostic in &run.source_diagnostics {
        println!("  [source] {diagnostic}");
    }
    if run.completion.candidates.is_empty() {
        println!("候选: 0（没有通过当前上下文和匹配规则的候选）");
    } else {
        println!("候选: {} 条", run.completion.candidates.len());
        for candidate in &run.completion.candidates {
            let source = if candidate.source.is_empty() {
                "engine"
            } else {
                candidate.source.as_str()
            };
            let reason = if candidate.source.starts_with("builtin") {
                "内置规格匹配"
            } else if candidate.source.is_empty() {
                "命令或会话快照匹配"
            } else {
                "用户规格或动态数据源匹配"
            };
            println!(
                "  {}\t{:?}\tsource={}\t{}",
                candidate.label, candidate.kind, source, reason
            );
        }
    }
    Ok(())
}

fn empty_check_report() -> CheckReport {
    CheckReport {
        files: Vec::new(),
        diagnostics: Vec::new(),
    }
}

fn check_catalog_directory(directory: &Path, explicit: bool) -> Result<CheckReport> {
    match Catalog::check_dir(directory) {
        Ok(report) => Ok(report),
        Err(CatalogLoadError::Io { source, .. })
            if !explicit && source.kind() == std::io::ErrorKind::NotFound =>
        {
            Ok(empty_check_report())
        }
        Err(error) => Err(load_catalog_error(error)),
    }
}

fn load_catalog_error(error: CatalogLoadError) -> anyhow::Error {
    match error {
        CatalogLoadError::Io { path, source } => {
            anyhow!("无法读取用户规格目录 {}: {source}", path.display())
        }
    }
}

fn print_check_report(directory: &Path, report: &CheckReport, json: bool) -> Result<()> {
    if json {
        let files = report
            .files
            .iter()
            .map(|file| {
                serde_json::json!({
                    "path": file.path,
                    "valid": file.valid,
                    "node_count": file.node_count,
                    "option_set_count": file.option_set_count,
                })
            })
            .collect::<Vec<_>>();
        let diagnostics = report
            .diagnostics
            .iter()
            .map(diagnostic_json)
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "directory": directory,
                "valid": report.is_valid(),
                "files": files,
                "diagnostics": diagnostics,
            }))?
        );
    } else {
        println!("规格检查: {}", directory.display());
        if report.files.is_empty() {
            println!("  没有用户 TOML 文件（按空目录处理）");
        }
        for file in &report.files {
            println!(
                "  {}\t{}\tnodes={}\toption_sets={}",
                if file.valid { "有效" } else { "无效" },
                file.path,
                file.node_count,
                file.option_set_count
            );
        }
        for diagnostic in &report.diagnostics {
            println!(
                "  [{}] {}: {}",
                diagnostic.severity, diagnostic.path, diagnostic.message
            );
        }
        println!(
            "结果: {}",
            if report.is_valid() {
                "有效"
            } else {
                "存在错误"
            }
        );
    }
    Ok(())
}

fn run_specs_command(command: SpecsCommand, config_path: Option<&Path>) -> Result<u32> {
    let settings = config::load(config_path)?;
    match command {
        SpecsCommand::Check { directory, json } => {
            let explicit = directory.is_some();
            let directory = directory.unwrap_or_else(|| catalog_directory(&settings, config_path));
            let report = check_catalog_directory(&directory, explicit)?;
            print_check_report(&directory, &report, json)?;
            Ok(if report.is_valid() { 0 } else { 1 })
        }
        SpecsCommand::List { directory, json } => {
            let directory = directory.unwrap_or_else(|| catalog_directory(&settings, config_path));
            let catalog = load_catalog(&directory)?;
            let summaries = catalog.list();
            if json {
                let nodes = summaries
                    .iter()
                    .map(|summary| {
                        serde_json::json!({
                            "path": summary.path,
                            "description": summary.description,
                            "source": summary.source,
                            "provider": summary.provider,
                            "child_count": summary.child_count,
                            "option_count": summary.option_count,
                            "examples": summary.examples,
                        })
                    })
                    .collect::<Vec<_>>();
                let diagnostics = catalog
                    .diagnostics()
                    .iter()
                    .map(diagnostic_json)
                    .collect::<Vec<_>>();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "directory": directory,
                        "count": nodes.len(),
                        "nodes": nodes,
                        "diagnostics": diagnostics,
                    }))?
                );
            } else {
                println!(
                    "有效规格: {}（{} 个上下文）",
                    directory.display(),
                    summaries.len()
                );
                for summary in summaries {
                    println!(
                        "{}\t{}\tsource={}\tchildren={}\toptions={}\tprovider={}",
                        summary.path,
                        summary.description,
                        summary.source,
                        summary.child_count,
                        summary.option_count,
                        summary.provider.as_deref().unwrap_or("无")
                    );
                }
                for diagnostic in catalog.diagnostics() {
                    println!(
                        "  [{}] {}: {}",
                        diagnostic.severity, diagnostic.path, diagnostic.message
                    );
                }
            }
            Ok(0)
        }
    }
}

fn command_available(name: &str) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    let mut names = vec![name.to_owned()];
    if cfg!(windows) {
        names.extend([
            format!("{name}.exe"),
            format!("{name}.cmd"),
            format!("{name}.bat"),
        ]);
    }
    env::split_paths(&path).any(|directory| {
        names
            .iter()
            .any(|candidate| directory.join(candidate).is_file())
    })
}

fn run_doctor(config_path: Option<&Path>) -> Result<u32> {
    let path = config_path
        .map(Path::to_path_buf)
        .unwrap_or_else(config::default_path);
    let settings = match config::load(config_path) {
        Ok(settings) => settings,
        Err(error) => {
            println!("ShellSense {}", env!("CARGO_PKG_VERSION"));
            println!("Config: {}", path.display());
            println!("Config status: error: {error:#}");
            println!("Key diagnostics: 无法读取或验证配置，因此未启用快捷键");
            return Ok(1);
        }
    };
    let specs = catalog_directory(&settings, config_path);
    let catalog = load_catalog(&specs)?;
    let report = match Catalog::check_dir(&specs) {
        Ok(report) => report,
        Err(CatalogLoadError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            empty_check_report()
        }
        Err(error) => return Err(load_catalog_error(error)),
    };
    let spec_errors = catalog
        .diagnostics()
        .iter()
        .filter(|diagnostic| {
            diagnostic.severity == shellsense::spec_catalog::DiagnosticSeverity::Error
        })
        .count();
    let learning_path = config::statistics_path();
    let learning_state = if settings.learning.enabled {
        if learning_path.is_file() {
            format!("enabled; file={}", learning_path.display())
        } else {
            format!(
                "enabled; file={} (not created yet)",
                learning_path.display()
            )
        }
    } else {
        "disabled".to_owned()
    };
    let (builtin_catalog_name, builtin_catalog_version) = Catalog::builtin_metadata();
    println!(
        "ShellSense {}\nPlatform: {} / {}\nConfig: {}\nCache: {}\nSpecs: {}\nEffective contexts: {}\nSpec files: {}\nSpec diagnostics: {}\nMax candidates: {}\nCompletion: fuzzy={} dynamic={}\nKeys: trigger={} native={} details={} refresh={} reload={} protocol={}\nKey status: valid\nLearning: {}\nData sources:",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        path.display(),
        config::cache_dir().display(),
        specs.display(),
        catalog.list().len(),
        report.files.len(),
        spec_errors,
        settings.completion.max_results,
        settings.completion.fuzzy,
        settings.completion.dynamic,
        settings.keys.trigger,
        settings.keys.native,
        settings.keys.details,
        settings.keys.refresh,
        settings.keys.reload,
        settings.keys.protocol_prefix,
        learning_state
    );
    println!(
        "Built-in catalog: {} v{}",
        builtin_catalog_name, builtin_catalog_version
    );
    if let Some(session) = env::var_os("SHELLSENSE_SESSION_DIR") {
        let session = PathBuf::from(session);
        if let Ok(bytes) = std::fs::read(session.join("adapter.json"))
            && let Ok(status) = serde_json::from_slice::<serde_json::Value>(&bytes)
        {
            println!(
                "Current adapter: ready={} protocol={} PSReadLine available={}",
                status["ready"], status["protocol_version"], status["psreadline"]
            );
            println!(
                "Current key availability: {}",
                status["capabilities"]["public_keys"]
            );
            println!("Internal handlers: {}", status["key_handlers"]);
            println!("Current transport: {}", status["transport"]);
        }
        if let Ok(bytes) = std::fs::read(session.join("data-sources.json"))
            && let Ok(status) = serde_json::from_slice::<serde_json::Value>(&bytes)
        {
            println!("最近返回的数据源状态（候选数量、完整性和诊断）: {status}");
        }
    } else {
        println!(
            "Current key bindings: run doctor inside ShellSense to inspect the active session."
        );
    }
    for (label, command) in [
        ("git", "git"),
        ("cargo", "cargo"),
        ("npm", "npm"),
        ("pnpm", "pnpm"),
        ("PowerShell", "pwsh"),
    ] {
        println!(
            "  {label}: {} (fixed offline provider; no script execution)",
            if command_available(command) {
                "available"
            } else {
                "not found; static candidates remain available"
            }
        );
    }
    if !catalog.diagnostics().is_empty() {
        println!("Spec diagnostics:");
        for diagnostic in catalog.diagnostics() {
            println!(
                "  [{}] {}: {}",
                diagnostic.severity, diagnostic.path, diagnostic.message
            );
        }
    }
    Ok(if spec_errors != 0 || !report.is_valid() {
        1
    } else {
        0
    })
}

fn execute() -> Result<u32> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Run {
        shell: "pwsh.exe".into(),
        no_profile: false,
        data_dir: None,
        trace: None,
        transport: host::Transport::Osc,
    }) {
        Command::Run {
            shell,
            no_profile,
            data_dir,
            trace,
            transport,
        } => host::run(host::RunOptions {
            shell,
            no_profile,
            config_path: cli.config,
            data_dir: data_dir.unwrap_or_else(config::cache_dir),
            trace_path: trace,
            transport,
        }),
        Command::Complete {
            line,
            cursor,
            cwd,
            json,
            explain,
        } => {
            let settings = config::load(cli.config.as_deref())?;
            let cwd = cwd.unwrap_or(std::env::current_dir()?);
            let cursor = cursor.unwrap_or(line.len());
            validate_cursor(&line, cursor)?;
            let run = complete_line(&settings, cli.config.as_deref(), &line, cursor, &cwd)?;
            if explain {
                print_completion_explanation(&run, json)?;
            } else if json {
                println!("{}", serde_json::to_string_pretty(&run.completion)?);
            } else {
                for candidate in run.completion.candidates {
                    println!("{}\t{}", candidate.label, candidate.description);
                }
            }
            Ok(0)
        }
        Command::Specs { command } => run_specs_command(command, cli.config.as_deref()),
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
        Command::Doctor => run_doctor(cli.config.as_deref()),
        Command::Learning {
            command: LearningCommand::Clear,
        } => {
            let path = config::statistics_path();
            let learning = Learning::new(path.clone());
            learning.clear().context("无法清除本地补全选择统计")?;
            drop(learning);
            println!("已清除本地补全选择统计: {}", path.display());
            Ok(0)
        }
        Command::BetaProbe {
            shell,
            samples,
            transport,
            no_descriptions,
            host_executable,
            output,
        } => {
            let report = shellsense::beta_metrics::run(
                &host_executable.unwrap_or(std::env::current_exe()?),
                &shell,
                samples,
                !no_descriptions,
                match transport {
                    host::Transport::Osc => "osc",
                    host::Transport::Pipe => "pipe",
                },
            )?;
            let text = serde_json::to_string_pretty(&report)?;
            if let Some(path) = output {
                if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, &text)?;
            }
            println!("{text}");
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
