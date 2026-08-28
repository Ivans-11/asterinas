use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, anyhow, bail};
use flate2::read::GzDecoder;
use regex::Regex;
use serde::Deserialize;

use crate::{
    HarnessInteraction, HostLaunchConfig, OsdkMode, TestArgs, TestHarness, Workspace,
    apply_arch_features, archive_qemu_logs, build_osdk_command,
    case::{self, Arch, LoadedHost},
    clear_qemu_logs,
    image::ImageStore,
    initramfs, merge_features, run_test_process,
};

#[derive(Debug, Clone, Deserialize)]
pub struct CaseManifest {
    pub arch: Arch,
    pub case: String,
    pub image: Option<String>,
    #[serde(default)]
    pub payload_files: Vec<String>,
    #[serde(default)]
    pub payload_extract_bzimage_elf: Vec<PayloadBzImageElf>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub success_regex: Vec<String>,
    #[serde(default)]
    pub fail_regex: Vec<String>,
    pub shell_prompt: Option<String>,
    pub shell_init_cmd: Option<String>,
    #[serde(default)]
    pub interactions: Vec<HarnessInteraction>,
    #[serde(default)]
    pub extra_features: Vec<String>,
    #[serde(default)]
    pub extra_qemu_args: Vec<String>,
    #[serde(default)]
    pub test_files: Vec<String>,
    /// Additional payload files downloaded from pinned external sources.
    #[serde(default)]
    pub payload_downloads: Vec<String>,
    /// Include the existing benchmark tool package in the initramfs.
    #[serde(default)]
    pub enable_benchmark_test: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PayloadDownloadSpec {
    pub url: String,
    pub sha256: String,
    /// Optional relative path in the generated payload image. Defaults to
    /// the catalog label.
    #[serde(default)]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PayloadDownloadCatalog {
    #[serde(default)]
    downloads: BTreeMap<String, PayloadDownloadSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PayloadBzImageElf {
    pub source: String,
    pub output: String,
}

fn default_timeout_secs() -> u64 {
    120
}

#[derive(Debug, Clone)]
pub struct LoadedCase {
    pub dir: PathBuf,
    pub host: LoadedHost,
    pub manifest: CaseManifest,
}

impl LoadedCase {
    pub fn key(&self) -> String {
        format!(
            "{}-control-{}",
            self.manifest.arch.as_str(),
            self.manifest.case
        )
    }
}

#[derive(Debug, Clone)]
pub struct StagedCase {
    pub loaded: LoadedCase,
    pub scheme: String,
    pub features: Vec<String>,
    pub rendered_qemu_args: Vec<String>,
}

#[derive(Debug, Clone)]
struct StagedDownload {
    target: String,
    path: PathBuf,
}

pub fn test(workspace: &Workspace, args: TestArgs) -> Result<()> {
    if args.guest.is_some() {
        bail!("--guest is only supported for static Axvisor tests");
    }

    let arch = args.arch.unwrap_or_default();
    let case_name = args.case.as_deref().unwrap_or("smoke");
    // Build the initramfs first. Control payloads may require binaries
    // produced by this exact build; expose files declared by the case to
    // payload staging instead of relying on a stale image-store bundle.
    let preload = resolve_case(&workspace.root, arch, case_name)?;
    let initramfs = initramfs::prepare_initramfs(
        &workspace.root,
        arch,
        &preload.manifest.test_files,
        preload.manifest.enable_benchmark_test,
    )?;
    let staged_case = stage(
        &workspace.root,
        &workspace.images_dir,
        &workspace.case_stage_root,
        arch,
        case_name,
    )?;
    let host_launch = HostLaunchConfig {
        scheme: staged_case.scheme.clone(),
        features: staged_case.features.clone(),
        rendered_qemu_args: staged_case.rendered_qemu_args.clone(),
    };
    let mut build = build_osdk_command(
        workspace,
        arch,
        initramfs.clone(),
        Some(&host_launch),
        None,
        OsdkMode::Build,
        args.mode,
    )?;
    println!("[axvisorctl] building Axvisor host-mode test target...");
    let status = build
        .status()
        .context("failed to launch cargo osdk build for host-mode test")?;
    if !status.success() {
        bail!("cargo osdk build failed with status {status}");
    }

    let mut invocation = build_osdk_command(
        workspace,
        arch,
        initramfs,
        Some(&host_launch),
        None,
        OsdkMode::Run,
        args.mode,
    )?;
    let harness = build_harness(&workspace.logs_dir, &staged_case)?;

    clear_qemu_logs(workspace)?;
    run_test_process(&mut invocation, harness)?;
    archive_qemu_logs(workspace, staged_case.loaded.key())?;
    Ok(())
}

pub fn stage(
    workspace_root: &Path,
    images_dir: &Path,
    case_stage_root: &Path,
    arch: Arch,
    name: &str,
) -> Result<StagedCase> {
    let loaded = resolve_case(workspace_root, arch, name)?;
    let image_store = ImageStore::new(images_dir.to_path_buf())?;
    let image_dir = loaded
        .manifest
        .image
        .as_deref()
        .map(|image| image_store.ensure_image(image))
        .transpose()?;
    let catalog = load_payload_download_catalog(workspace_root)?;
    let downloads = loaded
        .manifest
        .payload_downloads
        .iter()
        .map(|label| {
            let artifact = catalog.downloads.get(label).ok_or_else(|| {
                anyhow!(
                    "payload download `{label}` is not defined in {}",
                    workspace_root
                        .join("test/axvisor/payload-downloads.toml")
                        .display()
                )
            })?;
            let target = artifact.target.clone().unwrap_or_else(|| label.clone());
            Ok(StagedDownload {
                target,
                path: image_store.ensure_download(label, &artifact.url, &artifact.sha256)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let payload_image = image_dir
        .as_deref()
        .map(|image_dir| build_payload_image(case_stage_root, &loaded, image_dir, &downloads))
        .transpose()?;
    let mut features = merge_features(
        &loaded.host.manifest.features,
        &loaded.manifest.extra_features,
    );
    apply_arch_features(loaded.manifest.arch, &mut features)?;
    let rendered_qemu_args = loaded
        .host
        .manifest
        .host_qemu_args
        .iter()
        .chain(loaded.manifest.extra_qemu_args.iter())
        .map(|arg| {
            render_token(
                arg,
                &loaded.dir,
                image_dir.as_deref(),
                payload_image.as_deref(),
                workspace_root,
            )
        })
        .collect::<Vec<_>>();
    ensure_tokens_resolved(&loaded, &rendered_qemu_args)?;

    Ok(StagedCase {
        scheme: loaded.host.manifest.scheme.clone(),
        features,
        rendered_qemu_args,
        loaded,
    })
}

fn load_payload_download_catalog(workspace_root: &Path) -> Result<PayloadDownloadCatalog> {
    let path = workspace_root.join("test/axvisor/payload-downloads.toml");
    if !path.is_file() {
        return Ok(PayloadDownloadCatalog::default());
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

pub fn build_harness(logs_dir: &Path, staged_case: &StagedCase) -> Result<TestHarness> {
    let qemu_log_prefix = staged_case.loaded.key();
    let success = compile_regex_list("success", &staged_case.loaded.manifest.success_regex)?;
    let failure = compile_regex_list("fail", &staged_case.loaded.manifest.fail_regex)?;
    if success.is_empty() {
        bail!(
            "control case `{}` does not define success_regex; test mode requires an explicit success condition",
            staged_case.loaded.key()
        );
    }

    Ok(TestHarness {
        timeout: std::time::Duration::from_secs(staged_case.loaded.manifest.timeout_secs),
        success,
        failure,
        shell_prompt: staged_case.loaded.manifest.shell_prompt.clone(),
        shell_init_cmd: staged_case.loaded.manifest.shell_init_cmd.clone(),
        interactions: staged_case.loaded.manifest.interactions.clone(),
        log_path: logs_dir.join(format!("{qemu_log_prefix}.run.log")),
        qemu_log_prefix,
    })
}

fn compile_regex_list(label: &str, patterns: &[String]) -> Result<Vec<Regex>> {
    patterns
        .iter()
        .map(|pattern| {
            Regex::new(pattern).with_context(|| format!("invalid {label} regex `{pattern}`"))
        })
        .collect()
}

fn build_payload_image(
    case_stage_root: &Path,
    loaded: &LoadedCase,
    image_dir: &Path,
    downloads: &[StagedDownload],
) -> Result<PathBuf> {
    let case_dir = case_stage_root.join(loaded.key());
    let payload_dir = case_dir.join("payload");
    if payload_dir.exists() {
        fs::remove_dir_all(&payload_dir)
            .with_context(|| format!("failed to remove {}", payload_dir.display()))?;
    }
    fs::create_dir_all(&payload_dir)
        .with_context(|| format!("failed to create {}", payload_dir.display()))?;
    copy_dir_contents(image_dir, &payload_dir)?;
    // A control case may provide small, case-specific payload assets (for
    // example a benchmark configuration) under `payload/`.  Keep these
    // assets in the generated ext2 image alongside files copied from the
    // named image store.
    let case_payload_dir = loaded.dir.join("payload");
    if case_payload_dir.is_dir() {
        copy_dir_contents(&case_payload_dir, &payload_dir)?;
    }
    for download in downloads {
        let target = payload_dir.join(&download.target);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&download.path, &target).with_context(|| {
            format!(
                "failed to stage downloaded payload {} as {}",
                download.path.display(),
                target.display()
            )
        })?;
    }
    let missing = loaded
        .manifest
        .payload_files
        .iter()
        .filter(|name| !payload_dir.join(name).is_file())
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!(
            "control payload `{}` is missing required files: {}",
            loaded.key(),
            missing.join(", ")
        );
    }
    extract_payload_bzimage_elfs(&payload_dir, &loaded.manifest.payload_extract_bzimage_elf)?;

    let image_path = case_dir.join("payload.ext2.img");
    build_ext2_image_from_dir(&payload_dir, &image_path, &loaded.manifest.payload_files)?;
    Ok(image_path)
}

fn extract_payload_bzimage_elfs(payload_dir: &Path, entries: &[PayloadBzImageElf]) -> Result<()> {
    for entry in entries {
        let source = payload_dir.join(&entry.source);
        let output = payload_dir.join(&entry.output);
        extract_bzimage_elf(&source, &output).with_context(|| {
            format!(
                "failed to extract ELF kernel {} from bzImage {}",
                output.display(),
                source.display()
            )
        })?;
    }
    Ok(())
}

fn extract_bzimage_elf(source: &Path, output: &Path) -> Result<()> {
    const GZIP_MAGIC: &[u8] = &[0x1f, 0x8b, 0x08];
    const ELF_MAGIC: &[u8] = b"\x7fELF";

    let image = fs::read(source).with_context(|| format!("failed to read {}", source.display()))?;
    for offset in image
        .windows(GZIP_MAGIC.len())
        .enumerate()
        .filter_map(|(offset, bytes)| (bytes == GZIP_MAGIC).then_some(offset))
    {
        let mut decoder = GzDecoder::new(&image[offset..]);
        let mut decoded = Vec::new();
        if decoder.read_to_end(&mut decoded).is_ok() && decoded.starts_with(ELF_MAGIC) {
            fs::write(output, decoded)
                .with_context(|| format!("failed to write {}", output.display()))?;
            return Ok(());
        }
    }

    bail!(
        "no gzip-compressed ELF payload found in {}",
        source.display()
    )
}

fn render_token(
    value: &str,
    case_dir: &Path,
    guest_dir: Option<&Path>,
    payload_image: Option<&Path>,
    workspace_root: &Path,
) -> String {
    let rendered = value
        .replace("{case_dir}", &case_dir.to_string_lossy())
        .replace("{workspace_root}", &workspace_root.to_string_lossy());
    let rendered = if let Some(guest_dir) = guest_dir {
        rendered.replace("{guest_dir}", &guest_dir.to_string_lossy())
    } else {
        rendered
    };
    if let Some(payload_image) = payload_image {
        rendered.replace("{control_payload_image}", &payload_image.to_string_lossy())
    } else {
        rendered
    }
}

fn ensure_tokens_resolved(loaded: &LoadedCase, qemu_args: &[String]) -> Result<()> {
    if qemu_args
        .iter()
        .any(|arg| arg.contains("{control_payload_image}"))
    {
        bail!(
            "control case `{}` uses {{control_payload_image}} but does not define `image`",
            loaded.key()
        );
    }
    Ok(())
}

fn resolve_case(workspace_root: &Path, arch: Arch, name: &str) -> Result<LoadedCase> {
    let arch_dir = workspace_root.join("test/axvisor").join(arch.as_str());
    let case_dir = arch_dir.join("control").join(name);
    let manifest_path = case_dir.join("case.toml");
    if !manifest_path.is_file() {
        bail!(
            "no Axvisor control case `{name}` found for arch {} under {}",
            arch.as_str(),
            case_dir.display()
        );
    }

    let manifest = load_case_manifest(&manifest_path)?;
    if manifest.arch != arch {
        bail!(
            "control case `{name}` declares arch {}, but was resolved as {}",
            manifest.arch.as_str(),
            arch.as_str()
        );
    }
    if manifest.case != name {
        bail!(
            "control case file {} declares case `{}`, expected `{name}`",
            manifest_path.display(),
            manifest.case
        );
    }

    let host = case::resolve_host(workspace_root, arch)?.ok_or_else(|| {
        anyhow!(
            "missing host.toml for Axvisor arch `{}` under {}",
            arch.as_str(),
            arch_dir.display()
        )
    })?;

    Ok(LoadedCase {
        dir: case_dir,
        host,
        manifest,
    })
}

fn load_case_manifest(path: &Path) -> Result<CaseManifest> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

fn copy_dir_contents(source: &Path, target: &Path) -> Result<()> {
    for entry in
        fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
    {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            fs::create_dir_all(&target_path)
                .with_context(|| format!("failed to create {}", target_path.display()))?;
            copy_dir_contents(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    source_path.display(),
                    target_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn build_ext2_image_from_dir(
    source_dir: &Path,
    image_path: &Path,
    required_files: &[String],
) -> Result<()> {
    fs::create_dir_all(
        image_path
            .parent()
            .ok_or_else(|| anyhow!("invalid payload image path {}", image_path.display()))?,
    )
    .with_context(|| format!("failed to create parent for {}", image_path.display()))?;
    if image_path.exists() || image_path.symlink_metadata().is_ok() {
        remove_path(image_path)?;
    }

    let image_size = estimate_ext2_image_size(source_dir)?;
    let mut truncate = Command::new("truncate");
    truncate
        .arg("-s")
        .arg(image_size.to_string())
        .arg(image_path);
    run_command_status(&mut truncate, "create control payload image")?;

    let mut mkfs = Command::new("mkfs.ext2");
    mkfs.arg("-F").arg(image_path);
    run_command_status(&mut mkfs, "format control payload image")?;

    let commands = collect_debugfs_write_commands(source_dir, Path::new(""))?;
    let mut debugfs = Command::new("debugfs");
    debugfs.arg("-w").arg("-f").arg("-").arg(image_path);
    debugfs.stdin(Stdio::piped());
    debugfs.stdout(Stdio::null());
    let mut child = debugfs
        .spawn()
        .context("failed to launch debugfs for control payload image")?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .context("failed to open debugfs stdin")?;
        stdin
            .write_all(commands.as_bytes())
            .context("failed to write debugfs commands")?;
    }
    let status = child
        .wait()
        .context("failed to wait for debugfs for control payload image")?;
    if status.success() {
        verify_control_payload_image(image_path, required_files)?;
        Ok(())
    } else {
        bail!("populate control payload image failed with status {status}")
    }
}

fn estimate_ext2_image_size(source_dir: &Path) -> Result<u64> {
    const MIN_IMAGE_SIZE: u64 = 64 * 1024 * 1024;
    const EXTRA_SPACE: u64 = 16 * 1024 * 1024;
    const ALIGN: u64 = 1024 * 1024;

    let payload_size = dir_size(source_dir)?;
    let wanted = payload_size
        .checked_mul(2)
        .and_then(|size| size.checked_add(EXTRA_SPACE))
        .ok_or_else(|| anyhow!("control payload size overflow"))?
        .max(MIN_IMAGE_SIZE);
    Ok(wanted.div_ceil(ALIGN) * ALIGN)
}

fn dir_size(path: &Path) -> Result<u64> {
    let mut total = 0u64;
    for entry in fs::read_dir(path).with_context(|| format!("failed to read {}", path.display()))? {
        let entry = entry?;
        let metadata = entry
            .metadata()
            .with_context(|| format!("failed to stat {}", entry.path().display()))?;
        if metadata.is_dir() {
            total = total
                .checked_add(dir_size(&entry.path())?)
                .ok_or_else(|| anyhow!("directory size overflow"))?;
        } else {
            total = total
                .checked_add(metadata.len())
                .ok_or_else(|| anyhow!("directory size overflow"))?;
        }
    }
    Ok(total)
}

fn verify_control_payload_image(image_path: &Path, required_files: &[String]) -> Result<()> {
    if required_files.is_empty() {
        return Ok(());
    }
    let mut debugfs = Command::new("debugfs");
    debugfs.arg("-R").arg("ls -l /").arg(image_path);
    let output = debugfs
        .output()
        .context("failed to inspect control payload image")?;
    if !output.status.success() {
        bail!(
            "inspect control payload image failed with status {}",
            output.status
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for name in required_files {
        if !stdout.contains(name) {
            bail!(
                "control payload image {} is missing {name}",
                image_path.display()
            );
        }
    }
    Ok(())
}

fn collect_debugfs_write_commands(source_dir: &Path, relative: &Path) -> Result<String> {
    let mut commands = String::new();
    for entry in fs::read_dir(source_dir)
        .with_context(|| format!("failed to read {}", source_dir.display()))?
    {
        let entry = entry?;
        let source_path = entry.path();
        let target_relative = relative.join(entry.file_name());
        let target_path = format!("/{}", target_relative.to_string_lossy());
        if entry.file_type()?.is_dir() {
            commands.push_str(&format!("mkdir {target_path}\n"));
            commands.push_str(&collect_debugfs_write_commands(
                &source_path,
                &target_relative,
            )?);
        } else {
            commands.push_str(&format!(
                "write {} {}\n",
                source_path.to_string_lossy(),
                target_path
            ));
        }
    }
    Ok(commands)
}

fn remove_path(path: &Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path).with_context(|| format!("failed to remove {}", path.display()))?;
    } else {
        fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

fn run_command_status(command: &mut Command, action: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("failed to {action}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("{action} failed with status {status}")
    }
}
