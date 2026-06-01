use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum, Deserialize)]
#[clap(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Arch {
    X86_64,
    Riscv64,
    Loongarch64,
}

impl Arch {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Riscv64 => "riscv64",
            Self::Loongarch64 => "loongarch64",
        }
    }

    pub fn default_scheme(self) -> &'static str {
        match self {
            Self::X86_64 => "axvisor-x86_64",
            Self::Riscv64 => "axvisor-riscv64",
            Self::Loongarch64 => "axvisor-loongarch64",
        }
    }
}

impl Default for Arch {
    fn default() -> Self {
        Self::X86_64
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct HostManifest {
    pub scheme: String,
    #[serde(default = "default_features")]
    pub features: Vec<String>,
    #[serde(default)]
    pub host_qemu_args: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CaseManifest {
    pub arch: Arch,
    pub guest: String,
    pub image: String,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub success_regex: Vec<String>,
    #[serde(default)]
    pub fail_regex: Vec<String>,
    pub shell_prompt: Option<String>,
    pub shell_init_cmd: Option<String>,
    #[serde(default)]
    pub extra_features: Vec<String>,
    #[serde(default)]
    pub extra_qemu_args: Vec<String>,
}

fn default_features() -> Vec<String> {
    vec!["axvisor".to_string()]
}

fn default_timeout_secs() -> u64 {
    120
}

#[derive(Debug, Clone)]
pub struct LoadedHost {
    pub manifest: HostManifest,
}

#[derive(Debug, Clone)]
pub struct LoadedCase {
    pub dir: PathBuf,
    pub host: LoadedHost,
    pub manifest: CaseManifest,
}

impl LoadedCase {
    pub fn key(&self) -> String {
        format!("{}-{}", self.manifest.arch.as_str(), self.manifest.guest)
    }
}

pub fn load_all_cases(workspace_root: &Path) -> Result<Vec<LoadedCase>> {
    let suite_root = workspace_root.join("test-suit/axvisor");
    let mut cases = Vec::new();
    if !suite_root.is_dir() {
        return Ok(cases);
    }

    for arch_entry in fs::read_dir(&suite_root)
        .with_context(|| format!("failed to read {}", suite_root.display()))?
    {
        let arch_entry = arch_entry?;
        if !arch_entry.file_type()?.is_dir() {
            continue;
        }
        let arch_dir = arch_entry.path();
        for guest_entry in fs::read_dir(arch_entry.path())
            .with_context(|| format!("failed to read {}", arch_entry.path().display()))?
        {
            let guest_entry = guest_entry?;
            if !guest_entry.file_type()?.is_dir() {
                continue;
            }
            let case_dir = guest_entry.path();
            let manifest_path = case_dir.join("case.toml");
            if !manifest_path.is_file() {
                continue;
            }
            let manifest = load_case_manifest(&manifest_path)?;
            let host = load_host_manifest(&arch_dir)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "missing host.toml for Axvisor arch `{}` under {}",
                    manifest.arch.as_str(),
                    arch_dir.display()
                )
            })?;
            if !case_dir.join("vm.toml").is_file() {
                bail!("missing vm.toml for case {}", case_dir.display());
            }
            cases.push(LoadedCase {
                dir: case_dir,
                host,
                manifest,
            });
        }
    }

    cases.sort_by(|a, b| a.key().cmp(&b.key()));
    Ok(cases)
}

pub fn resolve_case(workspace_root: &Path, arch: Option<Arch>, guest: &str) -> Result<LoadedCase> {
    let mut matches = load_all_cases(workspace_root)?
        .into_iter()
        .filter(|case| case.manifest.guest == guest)
        .collect::<Vec<_>>();

    if let Some(arch) = arch {
        matches.retain(|case| case.manifest.arch == arch);
    }

    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => {
            let arch_hint = arch
                .map(|value| format!(" for arch {}", value.as_str()))
                .unwrap_or_default();
            bail!("no Axvisor case found for guest `{guest}`{arch_hint}")
        }
        _ => {
            let labels = matches
                .iter()
                .map(|case| case.key())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("guest `{guest}` is ambiguous. Matching cases: {labels}. Pass --arch explicitly.")
        }
    }
}

pub fn resolve_host(workspace_root: &Path, arch: Arch) -> Result<Option<LoadedHost>> {
    let arch_dir = workspace_root.join("test-suit/axvisor").join(arch.as_str());
    load_host_manifest(&arch_dir)
}

fn load_host_manifest(arch_dir: &Path) -> Result<Option<LoadedHost>> {
    let manifest_path = arch_dir.join("host.toml");
    if !manifest_path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let manifest: HostManifest = toml::from_str(&text)
        .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    Ok(Some(LoadedHost { manifest }))
}

fn load_case_manifest(path: &Path) -> Result<CaseManifest> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}
