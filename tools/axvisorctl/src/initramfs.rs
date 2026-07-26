use std::{
    env,
    ffi::OsString,
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};

use crate::case::Arch;

pub fn prepare_initramfs(
    workspace_root: &Path,
    arch: Arch,
    test_files: &[String],
) -> Result<PathBuf> {
    let test_dir = workspace_root.join("test/initramfs");
    let build_dir = test_dir.join("build");
    let initramfs_link = build_dir.join(arch.as_str()).join("initramfs.cpio.gz");

    let mut build = Command::new("make");
    build
        .current_dir(&test_dir)
        .arg(format!("TARGET_ARCH={}", arch.as_str()))
        .arg(format!("AXVISOR_TEST_FILES={}", test_files.join(",")))
        .arg("BENCHMARK=none")
        .arg("build")
        .stdin(Stdio::null());
    if let Some(path) = augmented_path()? {
        build.env("PATH", path);
    }
    run_command(&mut build, "build test initramfs")?;

    let final_artifact = if should_slim_initramfs(arch) {
        build_slim_initramfs(&build_dir, arch)?
    } else {
        build_dir
            .join("initramfs.cpio.gz")
            .canonicalize()
            .with_context(|| {
                format!(
                    "failed to resolve {}",
                    build_dir.join("initramfs.cpio.gz").display()
                )
            })?
    };

    let arch_dir = build_dir.join(arch.as_str());
    fs::create_dir_all(&arch_dir)
        .with_context(|| format!("failed to create {}", arch_dir.display()))?;
    remove_path_if_exists(&initramfs_link)?;
    symlink(&final_artifact, &initramfs_link).with_context(|| {
        format!(
            "failed to create symlink {} -> {}",
            initramfs_link.display(),
            final_artifact.display()
        )
    })?;

    Ok(initramfs_link)
}

fn build_slim_initramfs(build_dir: &Path, arch: Arch) -> Result<PathBuf> {
    let source_root = build_dir
        .join("initramfs")
        .canonicalize()
        .with_context(|| {
            format!(
                "failed to resolve {}",
                build_dir.join("initramfs").display()
            )
        })?;
    let arch_dir = build_dir.join(arch.as_str());
    let artifact = arch_dir.join("axvisor-initramfs.cpio.gz");
    let tmp_artifact = arch_dir.join("axvisor-initramfs.cpio.gz.tmp");
    fs::create_dir_all(&arch_dir)
        .with_context(|| format!("failed to create {}", arch_dir.display()))?;
    remove_path_if_exists(&tmp_artifact)?;

    let script = format!(
        "find . \\\n  \\( \\\n    -path './nix/store/*/share/i18n' -o -path './nix/store/*/share/i18n/*' -o \\\n    -path './nix/store/*/share/locale' -o -path './nix/store/*/share/locale/*' -o \\\n    -path './nix/store/*/lib/gconv' -o -path './nix/store/*/lib/gconv/*' -o \\\n    -path './nix/store/*/lib/locale' -o -path './nix/store/*/lib/locale/*' \\\n  \\) -prune -o -print0 | \\\n  LC_ALL=C sort -z | cpio --null -o -H newc --quiet | gzip -n > {}",
        sh_quote(tmp_artifact.to_string_lossy().as_ref())
    );
    let mut command = Command::new("bash");
    command
        .arg("-lc")
        .arg(script)
        .current_dir(&source_root)
        .stdin(Stdio::null());
    if let Some(path) = augmented_path()? {
        command.env("PATH", path);
    }
    run_command(&mut command, "slim initramfs for Axvisor")?;
    fs::rename(&tmp_artifact, &artifact).with_context(|| {
        format!(
            "failed to move {} to {}",
            tmp_artifact.display(),
            artifact.display()
        )
    })?;
    Ok(artifact)
}

fn should_slim_initramfs(arch: Arch) -> bool {
    match env::var("AXVISOR_SLIM_INITRAMFS") {
        Ok(value) => value == "1",
        Err(_) => arch == Arch::Riscv64,
    }
}

fn augmented_path() -> Result<Option<OsString>> {
    if command_exists("nix-build") {
        return Ok(None);
    }

    let mut segments = Vec::new();
    if let Some(path) = env::var_os("PATH") {
        segments.extend(env::split_paths(&path));
    }

    if let Ok(home) = env::var("HOME") {
        let profile_bin = PathBuf::from(home).join(".nix-profile/bin");
        if profile_bin.join("nix-build").is_file() {
            segments.insert(0, profile_bin);
        }
    }

    let default_profile = PathBuf::from("/nix/var/nix/profiles/default/bin");
    if default_profile.join("nix-build").is_file()
        && !segments.iter().any(|entry| entry == &default_profile)
    {
        segments.insert(0, default_profile);
    }

    if segments
        .iter()
        .all(|entry| !entry.join("nix-build").is_file())
    {
        bail!("missing nix-build in PATH; make sure Nix is installed and initialized");
    }

    env::join_paths(segments)
        .map(Some)
        .context("failed to construct PATH for Nix-aware initramfs build")
}

fn command_exists(name: &str) -> bool {
    env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .any(|path| path.join(name).is_file())
}

fn run_command(command: &mut Command, action: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("failed to {action}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("{action} failed with status {status}")
    }
}

fn remove_path_if_exists(path: &Path) -> Result<()> {
    if !path.exists() && path.symlink_metadata().is_err() {
        return Ok(());
    }
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path).with_context(|| format!("failed to remove {}", path.display()))?;
    } else {
        fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
