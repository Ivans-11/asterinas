use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

use anyhow::{Context, Result};

pub fn new_osdk_command(workspace_root: &Path, subcommand: &str) -> Result<Command> {
    if std::env::var("AXVISOR_USE_INSTALLED_OSDK").ok().as_deref() == Some("1") {
        let mut command = Command::new("cargo");
        command.arg("osdk").arg(subcommand);
        return Ok(command);
    }

    let local_bin = workspace_root.join("osdk/target/debug/cargo-osdk");
    if local_bin.is_file() && !osdk_source_is_newer(workspace_root, &local_bin)? {
        let mut command = Command::new(local_bin);
        command.arg("osdk").arg(subcommand);
        return Ok(command);
    }

    let mut command = Command::new("cargo");
    command
        .arg("run")
        .arg("--manifest-path")
        .arg(workspace_root.join("osdk/Cargo.toml"))
        .arg("--")
        .arg("osdk")
        .arg(subcommand)
        .env("OSDK_LOCAL_DEV", "1");
    Ok(command)
}

fn osdk_source_is_newer(workspace_root: &Path, local_bin: &Path) -> Result<bool> {
    let binary_mtime = fs::metadata(local_bin)
        .with_context(|| format!("failed to stat {}", local_bin.display()))?
        .modified()
        .with_context(|| format!("failed to read mtime for {}", local_bin.display()))?;
    for path in tracked_osdk_paths(workspace_root) {
        if source_newer_than(&path, binary_mtime)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn tracked_osdk_paths(workspace_root: &Path) -> [PathBuf; 3] {
    [
        workspace_root.join("osdk/src"),
        workspace_root.join("osdk/Cargo.toml"),
        workspace_root.join("osdk/Cargo.lock"),
    ]
}

fn source_newer_than(path: &Path, binary_mtime: SystemTime) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    if path.is_file() {
        let modified = fs::metadata(path)
            .with_context(|| format!("failed to stat {}", path.display()))?
            .modified()
            .with_context(|| format!("failed to read mtime for {}", path.display()))?;
        return Ok(modified > binary_mtime);
    }

    for entry in fs::read_dir(path).with_context(|| format!("failed to read {}", path.display()))? {
        let entry = entry?;
        if source_newer_than(&entry.path(), binary_mtime)? {
            return Ok(true);
        }
    }
    Ok(false)
}
