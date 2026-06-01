mod case;
mod image;
mod initramfs;
mod osdk;

use std::{
    env, fs,
    io::{self, Read, Write},
    os::unix::{fs::symlink, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use case::{Arch, LoadedCase};
use clap::{Args, Parser, Subcommand};
use image::ImageStore;
use regex::Regex;

const MATCH_DRAIN_DURATION: Duration = Duration::from_millis(500);
const MAX_MATCH_WINDOW_BYTES: usize = 2048;

#[derive(Parser)]
#[command(name = "axvisorctl")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Run(RunArgs),
    Test(TestArgs),
}

#[derive(Args)]
struct RunArgs {
    #[arg(long)]
    arch: Option<Arch>,
    #[arg(long)]
    guest: Option<String>,
}

#[derive(Args)]
struct TestArgs {
    #[arg(long)]
    arch: Option<Arch>,
    #[arg(long)]
    guest: String,
}

#[derive(Debug, Clone)]
struct Workspace {
    root: PathBuf,
    target_dir: PathBuf,
    images_dir: PathBuf,
    case_stage_root: PathBuf,
    logs_dir: PathBuf,
    default_vdso_dir: PathBuf,
}

impl Workspace {
    fn locate() -> Result<Self> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| anyhow!("failed to resolve Asterinas workspace root"))?
            .to_path_buf();
        let target_dir = root.join("target/axvisor");
        Ok(Self {
            images_dir: target_dir.join("images"),
            case_stage_root: target_dir.join("cases"),
            logs_dir: target_dir.join("logs"),
            default_vdso_dir: root.join("../linux_vdso"),
            root,
            target_dir,
        })
    }
}

#[derive(Debug, Clone)]
struct StagedCase {
    loaded: LoadedCase,
    vmconfig: PathBuf,
    image_dir: PathBuf,
    rendered_qemu_args: Vec<String>,
}

#[derive(Debug)]
struct TestHarness {
    timeout: Duration,
    success: Vec<Regex>,
    failure: Vec<Regex>,
    shell_prompt: Option<String>,
    shell_init_cmd: Option<String>,
    log_path: PathBuf,
    qemu_log_prefix: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamMatchKind {
    Success,
    Failure,
}

#[derive(Debug, Clone)]
struct StreamMatch {
    kind: StreamMatchKind,
    matched_regex: String,
    deadline: Instant,
}

struct ByteStreamMatcher {
    success_regex: Vec<Regex>,
    fail_regex: Vec<Regex>,
    match_buf: Vec<u8>,
    matched: Option<StreamMatch>,
}

impl ByteStreamMatcher {
    fn new(success_regex: Vec<Regex>, fail_regex: Vec<Regex>) -> Self {
        Self {
            success_regex,
            fail_regex,
            match_buf: Vec::with_capacity(MAX_MATCH_WINDOW_BYTES),
            matched: None,
        }
    }

    fn observe_byte(&mut self, byte: u8) -> Option<StreamMatch> {
        if self.matched.is_some() {
            return None;
        }

        self.match_buf.push(byte);
        if self.match_buf.len() > MAX_MATCH_WINDOW_BYTES {
            let overflow = self.match_buf.len() - MAX_MATCH_WINDOW_BYTES;
            self.match_buf.drain(..overflow);
        }

        let text = String::from_utf8_lossy(&self.match_buf);
        let text = strip_ansi_escape_sequences(&text);
        let matched = self
            .fail_regex
            .iter()
            .find(|regex| regex.is_match(&text))
            .map(|regex| StreamMatch {
                kind: StreamMatchKind::Failure,
                matched_regex: regex.as_str().to_string(),
                deadline: Instant::now() + MATCH_DRAIN_DURATION,
            })
            .or_else(|| {
                self.success_regex
                    .iter()
                    .find(|regex| regex.is_match(&text))
                    .map(|regex| StreamMatch {
                        kind: StreamMatchKind::Success,
                        matched_regex: regex.as_str().to_string(),
                        deadline: Instant::now() + MATCH_DRAIN_DURATION,
                    })
            });

        if let Some(matched) = matched {
            self.matched = Some(matched.clone());
            Some(matched)
        } else {
            None
        }
    }

    fn matched(&self) -> Option<&StreamMatch> {
        self.matched.as_ref()
    }

    fn should_stop(&self) -> bool {
        self.matched
            .as_ref()
            .is_some_and(|matched| Instant::now() >= matched.deadline)
    }
}

struct ShellAutoInitMatcher {
    shell_prompt: String,
    shell_init_cmd: Vec<u8>,
    history: Vec<u8>,
    triggered: bool,
}

impl ShellAutoInitMatcher {
    fn new(shell_prompt: Option<String>, shell_init_cmd: Option<String>) -> Option<Self> {
        match (shell_prompt, shell_init_cmd) {
            (Some(shell_prompt), Some(shell_init_cmd)) => Some(Self {
                history: Vec::with_capacity(shell_prompt.len().max(64)),
                shell_prompt,
                shell_init_cmd: prepare_shell_init_cmd(&shell_init_cmd),
                triggered: false,
            }),
            _ => None,
        }
    }

    fn observe_byte(&mut self, byte: u8) -> Option<Vec<u8>> {
        if self.triggered {
            return None;
        }

        self.history.push(byte);
        let max_len = self.shell_prompt.len().max(64) * 8;
        if self.history.len() > max_len {
            let excess = self.history.len() - max_len;
            self.history.drain(..excess);
        }

        if String::from_utf8_lossy(&self.history).contains(&self.shell_prompt) {
            self.triggered = true;
            Some(self.shell_init_cmd.clone())
        } else {
            None
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let workspace = Workspace::locate()?;

    match cli.command {
        Commands::Run(args) => run_command(&workspace, args),
        Commands::Test(args) => test_command(&workspace, args),
    }
}

fn run_command(workspace: &Workspace, args: RunArgs) -> Result<()> {
    fs::create_dir_all(&workspace.target_dir)
        .with_context(|| format!("failed to create {}", workspace.target_dir.display()))?;
    let initramfs =
        initramfs::prepare_initramfs(&workspace.root, resolve_run_arch(workspace, &args)?)?;
    let staged_case = match args.guest.as_deref() {
        Some(guest) => Some(stage_case(workspace, args.arch, guest)?),
        None => None,
    };

    let arch = staged_case
        .as_ref()
        .map(|case| case.loaded.manifest.arch)
        .or(args.arch)
        .unwrap_or_default();
    let mut invocation = build_osdk_command(
        workspace,
        arch,
        initramfs,
        staged_case.as_ref(),
        OsdkMode::Run,
    )?;

    clear_qemu_logs(workspace)?;
    println!("arch: {}", arch.as_str());
    if let Some(case) = staged_case.as_ref() {
        println!("guest: {}", case.loaded.manifest.guest);
        println!("image: {}", case.loaded.manifest.image);
        println!("image dir: {}", case.image_dir.display());
        println!("vmconfig: {}", case.vmconfig.display());
    } else {
        println!("guest: <host-only>");
    }

    let status = invocation
        .status()
        .context("failed to launch cargo osdk run")?;
    archive_qemu_logs(
        workspace,
        staged_case
            .as_ref()
            .map_or(arch.as_str().to_string(), |case| case.loaded.key()),
    )?;
    if status.success() {
        Ok(())
    } else {
        bail!("cargo osdk run failed with status {status}")
    }
}

fn test_command(workspace: &Workspace, args: TestArgs) -> Result<()> {
    fs::create_dir_all(&workspace.target_dir)
        .with_context(|| format!("failed to create {}", workspace.target_dir.display()))?;
    let staged_case = stage_case(workspace, args.arch, &args.guest)?;
    let initramfs =
        initramfs::prepare_initramfs(&workspace.root, staged_case.loaded.manifest.arch)?;
    let mut build = build_osdk_command(
        workspace,
        staged_case.loaded.manifest.arch,
        initramfs.clone(),
        Some(&staged_case),
        OsdkMode::Build,
    )?;
    println!("[axvisorctl] building Axvisor test target...");
    let status = build
        .status()
        .context("failed to launch cargo osdk build for test")?;
    if !status.success() {
        bail!("cargo osdk build failed with status {status}");
    }

    let mut invocation = build_osdk_command(
        workspace,
        staged_case.loaded.manifest.arch,
        initramfs,
        Some(&staged_case),
        OsdkMode::Run,
    )?;
    let harness = build_test_harness(workspace, &staged_case)?;

    clear_qemu_logs(workspace)?;
    run_test_process(&mut invocation, harness)?;
    archive_qemu_logs(workspace, staged_case.loaded.key())?;
    Ok(())
}

fn resolve_run_arch(workspace: &Workspace, args: &RunArgs) -> Result<Arch> {
    if let Some(guest) = args.guest.as_deref() {
        Ok(case::resolve_case(&workspace.root, args.arch, guest)?
            .manifest
            .arch)
    } else {
        Ok(args.arch.unwrap_or_default())
    }
}

fn stage_case(workspace: &Workspace, arch: Option<Arch>, guest: &str) -> Result<StagedCase> {
    let loaded = case::resolve_case(&workspace.root, arch, guest)?;
    let image_dir =
        ImageStore::new(workspace.images_dir.clone())?.ensure_image(&loaded.manifest.image)?;
    let staged_dir = workspace.case_stage_root.join(loaded.key());
    if staged_dir.exists() {
        fs::remove_dir_all(&staged_dir)
            .with_context(|| format!("failed to remove {}", staged_dir.display()))?;
    }
    fs::create_dir_all(&staged_dir)
        .with_context(|| format!("failed to create {}", staged_dir.display()))?;
    copy_case_assets(&loaded.dir, &staged_dir)?;

    let guest_link = staged_dir.join("guest");
    if guest_link.exists() || guest_link.symlink_metadata().is_ok() {
        remove_path(&guest_link)?;
    }
    symlink(&image_dir, &guest_link).with_context(|| {
        format!(
            "failed to create guest image link {} -> {}",
            guest_link.display(),
            image_dir.display()
        )
    })?;

    let rendered_qemu_args = loaded
        .manifest
        .extra_qemu_args
        .iter()
        .map(|arg| render_case_token(arg, &staged_dir, &image_dir, &workspace.root))
        .collect();

    Ok(StagedCase {
        vmconfig: staged_dir.join("vm.toml"),
        image_dir,
        rendered_qemu_args,
        loaded,
    })
}

#[derive(Debug, Clone, Copy)]
enum OsdkMode {
    Build,
    Run,
}

fn build_osdk_command(
    workspace: &Workspace,
    arch: Arch,
    initramfs: PathBuf,
    staged_case: Option<&StagedCase>,
    mode: OsdkMode,
) -> Result<Command> {
    let mut command = osdk::new_osdk_command(
        &workspace.root,
        match mode {
            OsdkMode::Build => "build",
            OsdkMode::Run => "run",
        },
    )?;
    let scheme = env::var("AXVISOR_SCHEME").unwrap_or_else(|_| {
        staged_case
            .map(|case| case.loaded.manifest.scheme.clone())
            .unwrap_or_else(|| arch.default_scheme().to_string())
    });
    let vdso_dir = env::var_os("VDSO_LIBRARY_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.default_vdso_dir.clone());
    if !vdso_dir.is_dir() {
        bail!("missing VDSO_LIBRARY_DIR: {}", vdso_dir.display());
    }

    command
        .current_dir(&workspace.root)
        .env("OSDK_TARGET_ARCH", arch.as_str())
        .env("VDSO_LIBRARY_DIR", &vdso_dir)
        .arg("--scheme")
        .arg(&scheme)
        .arg("--features")
        .arg("axvisor")
        .arg("--initramfs")
        .arg(&initramfs);

    if let Some(case) = staged_case {
        command.env("AXVISOR_VM_CONFIGS", &case.vmconfig);
    }

    if matches!(mode, OsdkMode::Run) {
        let qemu_args = compose_qemu_args(staged_case)?;
        if !qemu_args.is_empty() {
            command.arg(format!("--qemu-args={qemu_args}"));
        }
    }

    Ok(command)
}

fn compose_qemu_args(staged_case: Option<&StagedCase>) -> Result<String> {
    let mut parts = Vec::new();
    if let Some(case) = staged_case {
        parts.extend(case.rendered_qemu_args.iter().cloned());
    }
    if let Ok(extra) = env::var("AXVISOR_EXTRA_QEMU_ARGS") {
        let extra = extra.trim();
        if !extra.is_empty() {
            parts.push(extra.to_string());
        }
    }
    Ok(parts.join(" "))
}

fn build_test_harness(workspace: &Workspace, staged_case: &StagedCase) -> Result<TestHarness> {
    if staged_case.loaded.manifest.success_regex.is_empty() {
        bail!(
            "case `{}` does not define success_regex; test mode requires an explicit success condition",
            staged_case.loaded.key()
        );
    }

    let success = staged_case
        .loaded
        .manifest
        .success_regex
        .iter()
        .map(|pattern| {
            Regex::new(pattern).with_context(|| format!("invalid success regex `{pattern}`"))
        })
        .collect::<Result<Vec<_>>>()?;
    let failure = staged_case
        .loaded
        .manifest
        .fail_regex
        .iter()
        .map(|pattern| {
            Regex::new(pattern).with_context(|| format!("invalid fail regex `{pattern}`"))
        })
        .collect::<Result<Vec<_>>>()?;
    let qemu_log_prefix = staged_case.loaded.key();

    Ok(TestHarness {
        timeout: Duration::from_secs(staged_case.loaded.manifest.timeout_secs),
        success,
        failure,
        shell_prompt: staged_case.loaded.manifest.shell_prompt.clone(),
        shell_init_cmd: staged_case.loaded.manifest.shell_init_cmd.clone(),
        log_path: workspace
            .logs_dir
            .join(format!("{qemu_log_prefix}.run.log")),
        qemu_log_prefix,
    })
}

fn run_test_process(command: &mut Command, harness: TestHarness) -> Result<()> {
    let TestHarness {
        timeout,
        success,
        failure,
        shell_prompt,
        shell_init_cmd,
        log_path,
        qemu_log_prefix,
    } = harness;
    fs::create_dir_all(
        log_path
            .parent()
            .ok_or_else(|| anyhow!("invalid log path {}", log_path.display()))?,
    )
    .with_context(|| format!("failed to create {}", log_path.display()))?;
    let mut log_file = fs::File::create(&log_path)
        .with_context(|| format!("failed to create {}", log_path.display()))?;

    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().context("failed to spawn cargo osdk run")?;
    let child_pid = child.id() as i32;
    let mut stdin = child.stdin.take();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("failed to capture child stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("failed to capture child stderr"))?;

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    spawn_reader(stdout, tx.clone());
    spawn_reader(stderr, tx);

    let start = Instant::now();
    let mut matcher = ByteStreamMatcher::new(success, failure);
    let mut shell_auto_init = ShellAutoInitMatcher::new(shell_prompt, shell_init_cmd);
    let mut stream_closed = 0usize;

    loop {
        if start.elapsed() > timeout {
            terminate_process_group(child_pid);
            let _ = child.wait();
            bail!(
                "Axvisor test `{}` timed out after {}s; log: {}",
                qemu_log_prefix,
                timeout.as_secs(),
                log_path.display()
            );
        }

        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(bytes) => {
                if bytes.is_empty() {
                    stream_closed += 1;
                } else {
                    io::stdout()
                        .write_all(&bytes)
                        .context("failed to mirror test output")?;
                    io::stdout().flush().ok();
                    log_file
                        .write_all(&bytes)
                        .with_context(|| format!("failed to write {}", log_path.display()))?;
                    log_file.flush().ok();

                    for byte in bytes {
                        if let Some(shell_auto_init) = shell_auto_init.as_mut()
                            && let Some(command) = shell_auto_init.observe_byte(byte)
                            && let Some(child_stdin) = stdin.as_mut()
                        {
                            child_stdin
                                .write_all(&command)
                                .context("failed to write shell init command")?;
                            child_stdin.flush().ok();
                            let printable = String::from_utf8_lossy(&command);
                            println!(
                                "[axvisorctl] injected guest test command: {}",
                                printable.trim_end()
                            );
                        }

                        let _ = matcher.observe_byte(byte);
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                stream_closed = 2;
            }
        }

        if matcher.should_stop() {
            return finalize_match(
                &mut child,
                child_pid,
                matcher
                    .matched()
                    .ok_or_else(|| anyhow!("matcher stopped without a match"))?,
                &qemu_log_prefix,
                &log_path,
            );
        }

        if stream_closed >= 2 {
            if let Some(matched) = matcher.matched() {
                return finalize_match(&mut child, child_pid, matched, &qemu_log_prefix, &log_path);
            }
            match child.try_wait().context("failed to poll cargo osdk run")? {
                Some(status) => {
                    bail!(
                        "Axvisor test `{}` exited before reaching success condition (status {status}); log: {}",
                        qemu_log_prefix,
                        log_path.display()
                    );
                }
                None => {
                    terminate_process_group(child_pid);
                    let _ = child.wait();
                    bail!(
                        "Axvisor test `{}` lost all output streams unexpectedly; log: {}",
                        qemu_log_prefix,
                        log_path.display()
                    );
                }
            }
        }
    }
}

fn finalize_match(
    child: &mut std::process::Child,
    child_pid: i32,
    matched: &StreamMatch,
    qemu_log_prefix: &str,
    log_path: &Path,
) -> Result<()> {
    if child
        .try_wait()
        .context("failed to poll cargo osdk run")?
        .is_none()
    {
        terminate_process_group(child_pid);
    }
    let _ = child.wait();
    match matched.kind {
        StreamMatchKind::Success => {
            println!(
                "[axvisorctl] test `{}` passed via `{}`; log: {}",
                qemu_log_prefix,
                matched.matched_regex,
                log_path.display()
            );
            Ok(())
        }
        StreamMatchKind::Failure => {
            bail!(
                "Axvisor test `{}` failed via `{}`; log: {}",
                qemu_log_prefix,
                matched.matched_regex,
                log_path.display()
            )
        }
    }
}

fn strip_ansi_escape_sequences(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == 0x1b
            && let Some(next) = bytes.get(index + 1)
            && *next == b'['
        {
            index += 2;
            while index < bytes.len() {
                let byte = bytes[index];
                index += 1;
                if (0x40..=0x7e).contains(&byte) {
                    break;
                }
            }
            continue;
        }

        output.push(bytes[index]);
        index += 1;
    }

    String::from_utf8_lossy(&output).into_owned()
}

fn prepare_shell_init_cmd(command: &str) -> Vec<u8> {
    let mut normalized = command.trim_end_matches(['\r', '\n']).as_bytes().to_vec();
    normalized.push(b'\n');
    normalized
}

fn spawn_reader<R>(mut reader: R, tx: mpsc::Sender<Vec<u8>>)
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    let _ = tx.send(Vec::new());
                    break;
                }
                Ok(read) => {
                    let _ = tx.send(buffer[..read].to_vec());
                }
                Err(_) => {
                    let _ = tx.send(Vec::new());
                    break;
                }
            }
        }
    });
}

fn terminate_process_group(pid: i32) {
    if pid <= 0 {
        return;
    }
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
}

fn copy_case_assets(src: &Path, dst: &Path) -> Result<()> {
    for entry in fs::read_dir(src).with_context(|| format!("failed to read {}", src.display()))? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        if name == "case.toml" {
            continue;
        }
        let target = dst.join(&name);
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            fs::create_dir_all(&target)
                .with_context(|| format!("failed to create {}", target.display()))?;
            copy_case_assets(&path, &target)?;
        } else if file_type.is_symlink() {
            let link_target = fs::read_link(&path)
                .with_context(|| format!("failed to read link {}", path.display()))?;
            symlink(&link_target, &target).with_context(|| {
                format!(
                    "failed to create link {} -> {}",
                    target.display(),
                    link_target.display()
                )
            })?;
        } else {
            fs::copy(&path, &target).with_context(|| {
                format!("failed to copy {} to {}", path.display(), target.display())
            })?;
        }
    }
    Ok(())
}

fn render_case_token(
    value: &str,
    case_dir: &Path,
    guest_dir: &Path,
    workspace_root: &Path,
) -> String {
    value
        .replace("{case_dir}", &case_dir.to_string_lossy())
        .replace("{guest_dir}", &guest_dir.to_string_lossy())
        .replace("{workspace_root}", &workspace_root.to_string_lossy())
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

fn clear_qemu_logs(workspace: &Workspace) -> Result<()> {
    for name in ["qemu.log", "qemu-serial.log"] {
        let path = workspace.root.join(name);
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
    }
    Ok(())
}

fn archive_qemu_logs(workspace: &Workspace, prefix: String) -> Result<()> {
    fs::create_dir_all(&workspace.logs_dir)
        .with_context(|| format!("failed to create {}", workspace.logs_dir.display()))?;
    for name in ["qemu.log", "qemu-serial.log"] {
        let src = workspace.root.join(name);
        if src.is_file() {
            let dst = workspace.logs_dir.join(format!("{prefix}.{name}"));
            fs::copy(&src, &dst).with_context(|| {
                format!("failed to copy {} to {}", src.display(), dst.display())
            })?;
        }
    }
    Ok(())
}
