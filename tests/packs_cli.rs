#![cfg(windows)]

use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Command};

fn run(appdata: &Path, repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new(env!("CARGO_BIN_EXE_blueberry"))
        .env("APPDATA", appdata)
        .current_dir(repo)
        .args(args)
        .output()?;
    ensure!(
        output.status.success(),
        "{:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

#[test]
fn approval_is_disabled_in_a_new_process_as_soon_as_the_pack_changes() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let repo = directory.path().join("project");
    let appdata = directory.path().join("appdata");
    fs::create_dir_all(repo.join(".git"))?;
    fs::create_dir_all(repo.join(".blueberry"))?;
    let file = repo.join(".blueberry/commands.toml");
    let original = "schema_version = 1\n[pack]\nid = 'team'\nname = 'Team'\nversion = '1'\n[[operations]]\nid = 'test'\nname = 'Test'\ntokens = ['cargo', 'test']\n";
    fs::write(&file, original)?;
    let digest = format!("{:x}", Sha256::digest(fs::read(&file)?));
    ensure!(
        run(&appdata, &repo, &["packs", "review"])?.contains(&digest),
        "review did not show the exact digest"
    );
    run(&appdata, &repo, &["packs", "approve", &digest])?;
    ensure!(
        run(&appdata, &repo, &["packs", "list"])?.contains("已启用"),
        "approved pack is not active"
    );
    let approvals = appdata.join("Blueberry/project-packs.json");
    let prior = fs::read(&approvals).context("approval file")?;
    fs::write(&file, original.replace("version = '1'", "version = '2'"))?;
    ensure!(
        run(&appdata, &repo, &["packs", "list"])?.contains("待重新审查"),
        "changed pack remained active"
    );
    ensure!(fs::read(&approvals)? == prior, "listing rewrote approvals");
    ensure!(
        run(&appdata, &repo, &["packs", "review"])?.contains("待批准"),
        "new contents were not presented for review"
    );
    run(&appdata, &repo, &["packs", "revoke"])?;
    ensure!(
        run(&appdata, &repo, &["packs", "list"])?.contains("尚未批准"),
        "revoke did not persist"
    );
    Ok(())
}
