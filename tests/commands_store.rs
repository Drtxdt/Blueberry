#![cfg(windows)]

use anyhow::{Context, Result, ensure};
use std::{fs, path::Path, process::Command};

fn blueberry(appdata: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_blueberry"));
    command.env("APPDATA", appdata);
    command
}

#[test]
fn independent_processes_keep_all_favorites() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut children = Vec::new();
    for index in 0..16 {
        children.push(
            blueberry(root)
                .args([
                    "hub",
                    "--add",
                    &format!("favorite-{index}"),
                    "--command",
                    "git status",
                ])
                .spawn()
                .with_context(|| format!("start writer {index}"))?,
        );
    }
    for (index, mut child) in children.into_iter().enumerate() {
        ensure!(child.wait()?.success(), "writer {index} failed");
    }
    let path = root.join("Blueberry/commands.toml");
    let document: toml::Value = toml::from_str(&fs::read_to_string(&path)?)?;
    let favorites = document["favorites"]
        .as_array()
        .context("favorites array")?;
    ensure!(favorites.len() == 16, "lost a concurrent writer");
    for index in 0..16 {
        let name = format!("favorite-{index}");
        ensure!(
            favorites
                .iter()
                .any(|favorite| favorite["name"].as_str() == Some(name.as_str())),
            "favorite {index} was lost"
        );
    }
    Ok(())
}

#[test]
fn corrupt_or_unreadable_store_is_never_replaced() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("Blueberry/commands.toml");
    fs::create_dir_all(path.parent().context("store parent")?)?;
    for contents in [
        "schema_version = 2\n[[templates]]\nname = '",
        "schema_version = 99\n",
    ] {
        fs::write(&path, contents)?;
        for action in [
            ["hub", "--add", "new", "--command", "cargo test"].as_slice(),
            ["hub", "--remove", "old"].as_slice(),
        ] {
            ensure!(
                !blueberry(directory.path()).args(action).status()?.success(),
                "invalid store was accepted"
            );
            ensure!(
                fs::read_to_string(&path)? == contents,
                "invalid store changed"
            );
        }
    }
    fs::remove_file(&path)?;
    fs::create_dir(&path)?;
    ensure!(
        !blueberry(directory.path())
            .args(["hub", "--add", "new", "--command", "cargo test"])
            .status()?
            .success(),
        "directory was accepted as a store"
    );
    ensure!(path.is_dir(), "unreadable store path changed");
    Ok(())
}

#[test]
fn removing_an_unknown_favorite_does_not_touch_the_file() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("Blueberry/commands.toml");
    fs::create_dir_all(path.parent().context("store parent")?)?;
    let original = b"# personal note\nschema_version = 2\ncustom = 'keep'\n";
    fs::write(&path, original)?;
    let modified = fs::metadata(&path)?.modified()?;
    ensure!(
        blueberry(directory.path())
            .args(["hub", "--remove", "missing"])
            .status()?
            .success(),
        "remove failed"
    );
    ensure!(fs::read(&path)? == original, "store bytes changed");
    ensure!(
        fs::metadata(&path)?.modified()? == modified,
        "store was rewritten"
    );
    Ok(())
}
