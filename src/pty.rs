use anyhow::{Context, Result};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::path::PathBuf;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::Path,
};

pub struct Session {
    pub master: Box<dyn MasterPty + Send>,
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Box<dyn Child + Send + Sync>,
}

#[cfg(windows)]
fn default_windows_module_paths(legacy: bool) -> Vec<PathBuf> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, ptr};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
        RegGetValueW,
    };
    fn read(hive: HKEY, key: &str) -> Option<OsString> {
        let key: Vec<_> = key.encode_utf16().chain([0]).collect();
        let name: Vec<_> = "PSModulePath".encode_utf16().chain([0]).collect();
        let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ;
        let mut bytes = 0;
        if unsafe {
            RegGetValueW(
                hive,
                key.as_ptr(),
                name.as_ptr(),
                flags,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut bytes,
            )
        } != 0
        {
            return None;
        }
        for _ in 0..3 {
            if !(2..=131072).contains(&bytes) || bytes % 2 != 0 {
                return None;
            }
            let mut value = vec![0u16; bytes as usize / 2];
            let status = unsafe {
                RegGetValueW(
                    hive,
                    key.as_ptr(),
                    name.as_ptr(),
                    flags,
                    ptr::null_mut(),
                    value.as_mut_ptr().cast(),
                    &mut bytes,
                )
            };
            if status == 234 {
                continue;
            }
            if status != 0 {
                return None;
            }
            let end = value.iter().position(|unit| *unit == 0)?;
            return Some(OsString::from_wide(&value[..end]));
        }
        None
    }
    let mut paths = Vec::new();
    if legacy {
        use windows_sys::Win32::{
            System::Com::CoTaskMemFree,
            UI::Shell::{
                FOLDERID_Documents, FOLDERID_ProgramFiles, KF_FLAG_DONT_VERIFY,
                SHGetKnownFolderPath,
            },
        };
        // An empty GUI environment has no reliable per-user path in the
        // registry: image builders can leave a different user's directory.
        // Use the current user's redirected known folder, as ConsoleHost does.
        for (folder, suffix) in [
            (FOLDERID_Documents, "WindowsPowerShell/Modules"),
            (FOLDERID_ProgramFiles, "WindowsPowerShell/Modules"),
        ] {
            let mut value = ptr::null_mut();
            if unsafe {
                SHGetKnownFolderPath(
                    &folder,
                    KF_FLAG_DONT_VERIFY as u32,
                    ptr::null_mut(),
                    &mut value,
                )
            } >= 0
                && !value.is_null()
            {
                let mut length = 0;
                while unsafe { *value.add(length) } != 0 {
                    length += 1;
                }
                let path =
                    OsString::from_wide(unsafe { std::slice::from_raw_parts(value, length) });
                paths.push(PathBuf::from(path).join(suffix));
            }
            unsafe { CoTaskMemFree(value.cast()) };
        }
    }
    for (hive, key) in [
        (HKEY_CURRENT_USER, "Environment"),
        (
            HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        ),
    ] {
        if let Some(value) = read(hive, key) {
            paths.extend(std::env::split_paths(&value));
        }
    }
    paths
}

/// PowerShell 7 sanitizes PSModulePath when it launches powershell.exe
/// directly; native intermediate launchers must do the same themselves.
pub fn module_search_paths(shell: &Path) -> Vec<PathBuf> {
    let legacy = shell
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("powershell.exe"));
    let module_path = std::env::var_os("PSModulePath").filter(|p| !p.is_empty());
    let inherited = if legacy && module_path.is_some() {
        std::env::var_os("WinPSModulePath").filter(|p| !p.is_empty())
    } else {
        None
    }
    .or(module_path);
    #[cfg(windows)]
    let use_defaults = inherited.is_none();
    let mut paths = inherited
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    // A GUI launcher need not inherit PSModulePath. If we pass only the
    // private editor root, ConsoleHost reconstructs its default path ahead
    // of it and eagerly loads stock PSReadLine before our bootstrap runs.
    // Include PSHOME/Modules so PowerShell retains the supplied ordering.
    let resolved = if shell.components().count() == 1 {
        std::env::var_os("PATH").and_then(|value| {
            std::env::split_paths(&value)
                .map(|root| root.join(shell))
                .find(|path| path.is_file())
        })
    } else {
        None
    };
    if let Some(parent) = resolved.as_deref().unwrap_or(shell).parent() {
        let modules = parent.join("Modules");
        if modules.is_dir() && !paths.iter().any(|path| path == &modules) {
            paths.push(modules);
        }
    }
    // Supplying PSHOME suppresses PowerShell's registry-path merge. Restore
    // that merge for an absent/empty inherited path, without overriding an
    // explicitly configured nonempty path or starting another shell process.
    #[cfg(windows)]
    if use_defaults {
        for path in default_windows_module_paths(legacy) {
            if !paths.iter().any(|existing| existing == &path) {
                paths.push(path);
            }
        }
    }
    if legacy {
        paths.retain(|path| {
            let normalized = path
                .to_string_lossy()
                .replace('/', "\\")
                .trim_end_matches('\\')
                .to_ascii_lowercase();
            !normalized.ends_with("\\powershell\\modules")
                && !path
                    .parent()
                    .is_some_and(|parent| parent.join("pwsh.exe").is_file())
        });
    }
    paths
}

/// Keep startup key configuration identical for the host and paired probe.
/// JSON remains available for older adapters; the five checked strings avoid
/// cold JSON parsing solely for initial key arbitration in current adapters.
pub fn key_environment(keys: &crate::config::KeyBindings) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::from([
        ("BLUEBERRY_PUBLIC_KEYS_VERSION".into(), "1".into()),
        (
            "BLUEBERRY_PUBLIC_KEYS".into(),
            serde_json::to_string(keys).expect("key strings serialize"),
        ),
    ]);
    for (name, value) in [
        ("TRIGGER", &keys.trigger),
        ("NATIVE", &keys.native),
        ("DETAILS", &keys.details),
        ("REFRESH", &keys.refresh),
        ("RELOAD", &keys.reload),
        ("SEARCH", &keys.search),
        ("RESOURCES", &keys.resources),
        ("HUB", &keys.hub),
    ] {
        environment.insert(format!("BLUEBERRY_PUBLIC_KEY_{name}"), value.clone());
    }
    environment
}

pub fn spawn(
    program: &Path,
    args: &[String],
    cwd: &Path,
    env: &BTreeMap<String, String>,
    rows: u16,
    cols: u16,
) -> Result<Session> {
    #[cfg(windows)]
    crate::conpty::ensure_loaded()?;
    let pair = native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("Cannot create a pseudoterminal")?;
    let mut command = CommandBuilder::new(program);
    // portable-pty refreshes registry environment values on Windows. Preserve
    // the actual launching session instead (virtualenvs, PATH edits, etc.).
    command.env_clear();
    for (key, value) in std::env::vars_os() {
        command.env(key, value);
    }
    command.args(args);
    command.cwd(cwd);
    for (key, value) in env {
        command.env(key, value);
    }
    // ConsoleHost can load PSReadLine before -Command runs. Put the selected
    // test module first in discovery instead of attempting to replace an
    // already loaded binary module with Import-Module -Force.
    if program.file_name().is_some_and(|name| {
        name.eq_ignore_ascii_case("powershell.exe") || name.eq_ignore_ascii_case("pwsh.exe")
    }) && (std::env::var("BLUEBERRY_NO_HISTORY").as_deref() == Ok("1")
        || env.get("BLUEBERRY_NO_HISTORY").map(String::as_str) == Some("1"))
        && let Some(module) =
            std::env::var_os("BLUEBERRY_TEST_PSREADLINE_MODULE").filter(|module| !module.is_empty())
    {
        let module = PathBuf::from(module);
        if let Some(base) = module.parent().and_then(Path::parent) {
            let root = if base.file_name().is_some_and(|name| name == "PSReadLine") {
                base.parent().unwrap_or(base)
            } else {
                base
            };
            let mut paths = vec![root.to_owned()];
            paths.extend(module_search_paths(program));
            command.env("PSModulePath", std::env::join_paths(paths)?);
        }
    }
    let child = pair
        .slave
        .spawn_command(command)
        .with_context(|| format!("Cannot start {}", program.display()))?;
    drop(pair.slave);
    let reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    Ok(Session {
        master: pair.master,
        reader,
        writer,
        child,
    })
}

pub fn ensure_integration(directory: &Path) -> Result<std::path::PathBuf> {
    std::fs::create_dir_all(directory)?;
    let content = include_str!("../shell/integration.ps1").replace(
        "([IO.File]::ReadAllText((Join-Path $PSScriptRoot 'legacy-json.cs')))",
        &format!("@'\n{}\n'@", include_str!("../shell/legacy-json.cs")),
    );
    #[cfg(windows)]
    let content = {
        content.replace(
            "BLUEBERRY_LEGACY_ASSEMBLY_BASE64",
            include_str!(concat!(env!("OUT_DIR"), "/legacy-json.base64")),
        )
    };
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hash);
    let path = directory.join(format!("integration-{:016x}.ps1", hash.finish()));
    if std::fs::read_to_string(&path).ok().as_deref() != Some(content.as_str()) {
        let temporary = directory.join(format!("integration.{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&temporary, &content)?;
        if let Err(error) = std::fs::rename(&temporary, &path) {
            let _ = std::fs::remove_file(&temporary);
            if std::fs::read_to_string(&path).ok().as_deref() != Some(content.as_str()) {
                return Err(error.into());
            }
        }
    }
    Ok(path)
}

pub fn shell_args(integration: &Path, no_profile: bool) -> Vec<String> {
    shell_args_with_editor(integration, no_profile, None)
}

pub fn shell_args_with_editor(
    integration: &Path,
    no_profile: bool,
    editor: Option<&Path>,
) -> Vec<String> {
    let mut args = vec!["-NoLogo".into(), "-NoExit".into()];
    if no_profile {
        args.push("-NoProfile".into());
    }
    let isolated = std::env::var("BLUEBERRY_NO_HISTORY").as_deref() == Ok("1");
    let module = if let Some(editor) = editor {
        Some(editor.to_string_lossy().into_owned())
    } else if isolated {
        std::env::var("BLUEBERRY_TEST_PSREADLINE_MODULE")
            .ok()
            .filter(|s| !s.is_empty())
    } else {
        None
    };
    let imports_module = module.is_some();
    let mut module_import = module
        .map(|path| {
            let path = path.replace('\'', "''");
            if editor.is_some() {
                // A profile may deliberately load another module. Keep that
                // assembly; the bridge verifies identity and reports fallback.
                format!("$blueberryDirectModule = Get-Module PSReadLine; if (-not $blueberryDirectModule) {{ try {{ Import-Module '{path}' -ErrorAction Stop }} catch {{ Import-Module PSReadLine -ErrorAction Stop }}; $blueberryDirectModule = Get-Module PSReadLine }}; ")
            } else {
                format!("if (-not (Get-Module PSReadLine)) {{ Import-Module '{path}' -ErrorAction Stop }}; if ((Get-Module PSReadLine).ModuleBase -ine (Split-Path -LiteralPath '{path}')) {{ throw 'Unexpected PSReadLine version loaded' }}; ")
            }
        })
        .unwrap_or_default();
    if isolated {
        // Keep a malformed development adapter from falling through to a
        // shell that writes a real user's history during isolated tests.
        if !imports_module {
            module_import.push_str("Import-Module PSReadLine; ");
        }
        module_import.push_str("Set-PSReadLineOption -HistorySaveStyle SaveNothing; ");
    }
    args.extend([
        "-Command".into(),
        format!(
            "if ($env:BLUEBERRY_DIRECT_TRACE -eq '1') {{ $blueberryDirectCommandEnter = [Diagnostics.Stopwatch]::GetTimestamp() }}; {module_import}if ($env:BLUEBERRY_DIRECT_TRACE -eq '1') {{ $blueberryDirectExplicitModuleReady = [Diagnostics.Stopwatch]::GetTimestamp() }}; $blueberrySourceTimer = $null; if ($env:BLUEBERRY_TRACE -eq '1') {{ $blueberrySourceTimer = [Diagnostics.Stopwatch]::StartNew() }}; . '{}'; if ($null -ne $blueberrySourceTimer) {{ Send-BlueberryTrace -Stage 'script_source' -DurationMs $blueberrySourceTimer.Elapsed.TotalMilliseconds }}",
            integration.to_string_lossy().replace('\'', "''")
        ),
    ]);
    args
}

// Let ConsoleHost run the bootstrap directly after its normal profile phase.
// A -Command wrapper plus dot-sourcing adds a second script invocation; putting
// the whole bootstrap into -Command also lengthens process creation measurably.
pub fn direct_shell_args(
    integration: &Path,
    no_profile: bool,
    editor: Option<&Path>,
) -> Vec<String> {
    let mut args = vec!["-NoLogo".into(), "-NoExit".into()];
    if no_profile {
        args.push("-NoProfile".into());
    }
    args.extend(["-File".into(), integration.to_string_lossy().into_owned()]);
    if let Some(editor) = editor {
        args.extend([
            "-EditorManifest".into(),
            editor.to_string_lossy().into_owned(),
        ]);
    }
    args
}

/// Prefer PowerShell 7 and fall back to the Windows inbox shell.
pub fn default_shell() -> PathBuf {
    if let Some(path) = std::env::var_os("BLUEBERRY_TEST_SHELL").filter(|path| !path.is_empty()) {
        return path.into();
    }
    let name = if cfg!(windows) { "pwsh.exe" } else { "pwsh" };
    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths) {
            let path = directory.join(name);
            if path.is_file() {
                return path;
            }
        }
    }
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        return PathBuf::from(root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    }
    name.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_public_key_snapshot_contains_every_adapter_chord() {
        let keys = crate::config::KeyBindings::default();
        let environment = key_environment(&keys);
        for name in [
            "TRIGGER",
            "NATIVE",
            "DETAILS",
            "REFRESH",
            "RELOAD",
            "SEARCH",
            "RESOURCES",
            "HUB",
        ] {
            assert!(
                environment
                    .get(&format!("BLUEBERRY_PUBLIC_KEY_{name}"))
                    .is_some_and(|value| !value.is_empty()),
                "missing adapter startup chord {name}"
            );
        }
        assert_eq!(environment["BLUEBERRY_PUBLIC_KEY_SEARCH"], keys.search);
    }

    #[cfg(windows)]
    #[test]
    fn extracted_adapter_embeds_compiled_legacy_json() {
        let directory = tempfile::tempdir().unwrap();
        let path = ensure_integration(directory.path()).unwrap();
        let source = std::fs::read_to_string(path).unwrap();
        assert!(!source.contains("$embeddedLegacyJson = 'BLUEBERRY_LEGACY_ASSEMBLY_BASE64'"));
        assert!(source.contains("[Reflection.Assembly]::Load([Convert]::FromBase64String"));
    }
}
