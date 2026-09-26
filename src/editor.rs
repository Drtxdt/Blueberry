//! Selection and content-addressed installation of build-time private editors.
use crate::host::PsReadLineVersion;
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/private_editor.rs"));

pub fn build_identity() -> serde_json::Value {
    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("../vendor/psreadline/upstream.json"))
            .expect("pinned editor metadata");
    value["files"] = serde_json::json!(EDITOR_FILES.iter().map(|(version,path,_,hash)|
        serde_json::json!({"version":version,"path":path,"sha256":hash})).collect::<Vec<_>>());
    value
}

pub struct SelectedEditor {
    pub module_root: PathBuf,
    pub manifest: PathBuf,
    pub dll_sha256: String,
}

fn manifest_version(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let text = if bytes.starts_with(&[0xff, 0xfe]) {
        String::from_utf16(
            &bytes[2..]
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>(),
        )
        .ok()?
    } else {
        String::from_utf8(bytes).ok()?
    };
    text.lines().find_map(|line| {
        let (name, value) = line.trim().split_once('=')?;
        if !name.trim().eq_ignore_ascii_case("ModuleVersion") {
            return None;
        }
        let value = value.trim();
        let quote = value.chars().next()?;
        if quote != '\'' && quote != '"' {
            return None;
        }
        let version = value[1..].split(quote).next()?;
        Some(if version == "2.0" {
            "2.0.0".into()
        } else {
            version.into()
        })
    })
}

fn discover(shell: &Path) -> Option<String> {
    if std::env::var("BLUEBERRY_NO_HISTORY").as_deref() == Ok("1")
        && let Some(path) = std::env::var_os("BLUEBERRY_TEST_PSREADLINE_MODULE")
    {
        return manifest_version(Path::new(&path));
    }
    let resolved = if shell.components().count() == 1 {
        std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|root| root.join(shell))
                .find(|path| path.is_file())
        })
    } else {
        None
    };
    let shell = resolved.as_deref().unwrap_or(shell);
    let legacy = shell
        .file_name()?
        .to_string_lossy()
        .eq_ignore_ascii_case("powershell.exe");
    let mut roots = crate::pty::module_search_paths(shell);
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        roots.push(PathBuf::from(program_files).join(if legacy {
            "WindowsPowerShell/Modules"
        } else {
            "PowerShell/Modules"
        }));
    }
    roots.push(shell.parent()?.join("Modules"));
    for root in roots {
        let module = root.join("PSReadLine");
        if let Some(version) = manifest_version(&module.join("PSReadLine.psd1")) {
            return Some(version);
        }
        let mut versions: Vec<_> = std::fs::read_dir(module)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|entry| manifest_version(&entry.ok()?.path().join("PSReadLine.psd1")))
            .filter_map(|version| {
                let parts = version
                    .split('.')
                    .map(str::parse::<u32>)
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .ok()?;
                Some((parts, version))
            })
            .collect();
        versions.sort();
        if let Some((_, version)) = versions.pop() {
            return Some(version);
        }
    }
    None
}

pub fn install_asset(path: &Path, bytes: &[u8]) -> Result<()> {
    if std::fs::read(path).ok().as_deref() == Some(bytes) {
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().context("asset parent")?)?;
    let staging = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&staging, bytes)?;
    if let Err(error) = std::fs::rename(&staging, path) {
        let _ = std::fs::remove_file(&staging);
        if std::fs::read(path).ok().as_deref() != Some(bytes) {
            return Err(error.into());
        }
    }
    Ok(())
}

pub fn select(shell: &Path, selection: PsReadLineVersion) -> Result<Option<SelectedEditor>> {
    let version = match selection {
        PsReadLineVersion::Auto => discover(shell),
        PsReadLineVersion::V200 => Some("2.0.0".into()),
        PsReadLineVersion::V245 => Some("2.4.5".into()),
    };
    let Some(version) = version.filter(|v| v == "2.0.0" || v == "2.4.5") else {
        return Ok(None);
    };
    let files: Vec<_> = EDITOR_FILES
        .iter()
        .filter(|(v, _, _, _)| *v == version)
        .collect();
    ensure!(
        !files.is_empty(),
        "private editor is missing from this build"
    );
    let mut digest = Sha256::new();
    for (_, path, _, hash) in &files {
        digest.update(path.as_bytes());
        digest.update(hash.as_bytes());
    }
    let module_root = std::env::temp_dir()
        .join("blueberry-runtime")
        .join(format!("{:x}", digest.finalize()));
    let module = module_root.join("PSReadLine").join(&version);
    let mut dll_sha256 = String::new();
    for (_, path, bytes, hash) in files {
        install_asset(&module.join(path), bytes)?;
        if *path == "Microsoft.PowerShell.PSReadLine.dll"
            || *path == "Microsoft.PowerShell.PSReadLine2.dll"
        {
            dll_sha256 = (*hash).into();
        }
    }
    Ok(Some(SelectedEditor {
        module_root,
        manifest: module.join("PSReadLine.psd1"),
        dll_sha256,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifest_version_is_literal_and_handles_old_version() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("PSReadLine.psd1");
        for (text, expected) in [
            ("ModuleVersion = '2.0'", Some("2.0.0")),
            ("ModuleVersion='2.4.5'", Some("2.4.5")),
            ("ModuleVersion = (Get-Version)", None),
        ] {
            std::fs::write(&file, text).unwrap();
            assert_eq!(manifest_version(&file).as_deref(), expected);
        }
    }
}
