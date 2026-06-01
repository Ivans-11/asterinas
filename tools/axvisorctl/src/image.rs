use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tar::Archive;
use xz2::read::XzDecoder;

const DEFAULT_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/rcore-os/tgosimages/refs/heads/main/registry/default.toml";
const DEFAULT_FALLBACK_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/rcore-os/tgosimages/refs/heads/main/registry/v0.0.25.toml";
const AUTO_SYNC_THRESHOLD_SECS: u64 = 60 * 60 * 24 * 7;
const REGISTRY_FILENAME: &str = "images.toml";
const LAST_SYNC_FILENAME: &str = ".last_sync";
const EXTRACTED_SHA256_FILENAME: &str = ".archive.sha256";

#[derive(Debug, Clone)]
pub struct ImageStore {
    root: PathBuf,
    client: Client,
}

impl ImageStore {
    pub fn new(root: PathBuf) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("failed to build image download client")?;
        Ok(Self { root, client })
    }

    pub fn ensure_image(&self, name: &str) -> Result<PathBuf> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("failed to create {}", self.root.display()))?;

        let extract_dir = self.root.join(name);
        let registry = match self.ensure_registry() {
            Ok(registry) => Some(registry),
            Err(err) if extract_dir.is_dir() => {
                eprintln!(
                    "warning: failed to refresh image registry ({err}); reusing cached {}",
                    extract_dir.display()
                );
                return Ok(extract_dir);
            }
            Err(err) => return Err(err),
        };

        let registry = registry.expect("registry must exist here");
        let entry = registry
            .find_latest(name)
            .ok_or_else(|| anyhow!("image `{name}` not found in local registry"))?;
        if extracted_archive_matches(&extract_dir, &entry.sha256)? {
            return Ok(extract_dir);
        }

        let archive_name = archive_name_from_url(&entry.url)?;
        let archive_path = self.root.join(archive_name);
        self.ensure_archive(entry, &archive_path)?;
        extract_archive(&archive_path, &extract_dir, &entry.sha256)?;
        Ok(extract_dir)
    }

    fn ensure_registry(&self) -> Result<ImageRegistry> {
        let registry_path = self.root.join(REGISTRY_FILENAME);
        let should_refresh = match read_last_sync_time(&self.root) {
            Some(last_sync) => {
                current_unix_timestamp()?.saturating_sub(last_sync) >= AUTO_SYNC_THRESHOLD_SECS
            }
            None => true,
        };

        if !registry_path.is_file() {
            self.sync_registry()?;
        } else if should_refresh {
            if let Err(err) = self.sync_registry() {
                eprintln!("warning: failed to refresh Axvisor image registry: {err}");
            }
        }

        let text = fs::read_to_string(&registry_path)
            .with_context(|| format!("failed to read {}", registry_path.display()))?;
        toml::from_str(&text)
            .with_context(|| format!("failed to parse {}", registry_path.display()))
    }

    fn sync_registry(&self) -> Result<()> {
        let fallback_registry = std::env::var("AXVISOR_REGISTRY_FALLBACK_URL")
            .unwrap_or_else(|_| DEFAULT_FALLBACK_REGISTRY_URL.to_string());
        let source =
            resolve_bootstrap_source(&self.client, DEFAULT_REGISTRY_URL, &fallback_registry)?;
        let registry = ImageRegistry::fetch_with_includes(&self.client, &source.url)?;
        let path = self.root.join(REGISTRY_FILENAME);
        let text = toml::to_string_pretty(&registry).context("failed to encode image registry")?;
        fs::write(&path, text).with_context(|| format!("failed to write {}", path.display()))?;
        write_last_sync_time(&self.root)?;
        Ok(())
    }

    fn ensure_archive(&self, entry: &ImageEntry, archive_path: &Path) -> Result<()> {
        if archive_path.is_file() && image_verify_sha256(archive_path, &entry.sha256)? {
            return Ok(());
        }
        if archive_path.exists() {
            fs::remove_file(archive_path)
                .with_context(|| format!("failed to remove {}", archive_path.display()))?;
        }

        let part_path = archive_path.with_extension(format!(
            "{}.part",
            archive_path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or_default()
        ));
        if part_path.exists() {
            fs::remove_file(&part_path)
                .with_context(|| format!("failed to remove {}", part_path.display()))?;
        }

        let mut response = self
            .client
            .get(&entry.url)
            .send()
            .with_context(|| format!("failed to download {}", entry.url))?
            .error_for_status()
            .with_context(|| format!("failed to download {}", entry.url))?;
        let mut file = fs::File::create(&part_path)
            .with_context(|| format!("failed to create {}", part_path.display()))?;
        io::copy(&mut response, &mut file)
            .with_context(|| format!("failed to write {}", part_path.display()))?;
        file.flush()
            .with_context(|| format!("failed to flush {}", part_path.display()))?;

        if !image_verify_sha256(&part_path, &entry.sha256)? {
            let _ = fs::remove_file(&part_path);
            bail!("downloaded image checksum mismatch for {}", entry.url);
        }

        fs::rename(&part_path, archive_path).with_context(|| {
            format!(
                "failed to move downloaded archive {} to {}",
                part_path.display(),
                archive_path.display()
            )
        })?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImageRegistry {
    images: Vec<ImageEntry>,
}

impl ImageRegistry {
    fn fetch_with_includes(client: &Client, url: &str) -> Result<Self> {
        use std::collections::{HashMap, HashSet, VecDeque};

        let mut queue = VecDeque::from([url.to_string()]);
        let mut seen = HashSet::new();
        let mut by_key = HashMap::<(String, String), ImageEntry>::new();

        while let Some(current_url) = queue.pop_front() {
            if !seen.insert(current_url.clone()) {
                continue;
            }

            let body = fetch_text(client, &current_url)?;
            let raw: RawRegistry = toml::from_str(&body)
                .with_context(|| format!("failed to parse registry {current_url}"))?;
            for include in raw.includes {
                queue.push_back(include.url);
            }
            for image in raw.images {
                by_key
                    .entry((image.name.clone(), image.version.clone()))
                    .or_insert(image);
            }
        }

        Ok(Self {
            images: by_key.into_values().collect(),
        })
    }

    fn find_latest(&self, name: &str) -> Option<&ImageEntry> {
        self.images
            .iter()
            .filter(|entry| entry.name == name)
            .max_by(|a, b| a.released_at.cmp(&b.released_at))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawRegistry {
    #[serde(default)]
    includes: Vec<IncludeEntry>,
    #[serde(default)]
    images: Vec<ImageEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IncludeEntry {
    url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImageEntry {
    name: String,
    version: String,
    released_at: Option<String>,
    description: String,
    sha256: String,
    arch: String,
    url: String,
}

#[derive(Debug, Clone)]
struct RegistrySource {
    url: String,
}

fn resolve_bootstrap_source(
    client: &Client,
    default_url: &str,
    fallback_url: &str,
) -> Result<RegistrySource> {
    match fetch_text(client, default_url) {
        Ok(body) => {
            let raw: RawRegistry = toml::from_str(&body)
                .with_context(|| format!("failed to parse registry {default_url}"))?;
            if let Some(url) = preferred_include_url(&raw.includes) {
                Ok(RegistrySource { url })
            } else {
                Ok(RegistrySource {
                    url: default_url.to_string(),
                })
            }
        }
        Err(default_err) => {
            fetch_text(client, fallback_url).with_context(|| {
                format!(
                    "failed to fetch default registry {default_url} and fallback registry {fallback_url}"
                )
            })?;
            eprintln!("warning: failed to fetch default registry {default_url}: {default_err}");
            Ok(RegistrySource {
                url: fallback_url.to_string(),
            })
        }
    }
}

fn preferred_include_url(includes: &[IncludeEntry]) -> Option<String> {
    let mut best: Option<(&IncludeEntry, (u64, u64, u64))> = None;
    for include in includes {
        let Some(version) = parse_registry_version(&include.url) else {
            continue;
        };
        if best.is_none_or(|(_, best_version)| version > best_version) {
            best = Some((include, version));
        }
    }
    best.map(|(include, _)| include.url.clone())
        .or_else(|| includes.last().map(|include| include.url.clone()))
}

fn parse_registry_version(url: &str) -> Option<(u64, u64, u64)> {
    let filename = url.rsplit('/').next()?;
    let version = filename.strip_prefix('v')?.strip_suffix(".toml")?;
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn fetch_text(client: &Client, url: &str) -> Result<String> {
    client
        .get(url)
        .send()
        .with_context(|| format!("failed to fetch {url}"))?
        .error_for_status()
        .with_context(|| format!("failed to fetch {url}"))?
        .text()
        .with_context(|| format!("failed to read {url}"))
}

fn archive_name_from_url(url: &str) -> Result<&str> {
    url.rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| anyhow!("invalid image url `{url}`"))
}

fn extract_archive(archive_path: &Path, extract_dir: &Path, sha256: &str) -> Result<()> {
    if extract_dir.exists() {
        fs::remove_dir_all(extract_dir)
            .with_context(|| format!("failed to remove {}", extract_dir.display()))?;
    }
    fs::create_dir_all(extract_dir)
        .with_context(|| format!("failed to create {}", extract_dir.display()))?;

    let file = fs::File::open(archive_path)
        .with_context(|| format!("failed to open {}", archive_path.display()))?;
    let filename = archive_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("invalid archive path {}", archive_path.display()))?;

    if filename.ends_with(".tar.gz") || filename.ends_with(".tgz") {
        let decoder = GzDecoder::new(file);
        let mut archive = Archive::new(decoder);
        archive
            .unpack(extract_dir)
            .with_context(|| format!("failed to unpack {}", archive_path.display()))?;
    } else if filename.ends_with(".tar.xz") || filename.ends_with(".txz") {
        let decoder = XzDecoder::new(file);
        let mut archive = Archive::new(decoder);
        archive
            .unpack(extract_dir)
            .with_context(|| format!("failed to unpack {}", archive_path.display()))?;
    } else if filename.ends_with(".tar") {
        let mut archive = Archive::new(file);
        archive
            .unpack(extract_dir)
            .with_context(|| format!("failed to unpack {}", archive_path.display()))?;
    } else {
        bail!("unsupported archive format: {}", archive_path.display());
    }

    fs::write(extract_dir.join(EXTRACTED_SHA256_FILENAME), sha256)
        .with_context(|| format!("failed to write {}", extract_dir.display()))?;
    Ok(())
}

fn extracted_archive_matches(extract_dir: &Path, expected_sha256: &str) -> Result<bool> {
    let sha_path = extract_dir.join(EXTRACTED_SHA256_FILENAME);
    if !extract_dir.is_dir() || !sha_path.is_file() {
        return Ok(false);
    }
    let actual = fs::read_to_string(&sha_path)
        .with_context(|| format!("failed to read {}", sha_path.display()))?;
    Ok(actual.trim() == expected_sha256)
}

fn image_verify_sha256(path: &Path, expected_sha256: &str) -> Result<bool> {
    let mut file =
        fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut buffer = [0u8; 8192];
    let mut hasher = Sha256::new();
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());
    Ok(actual == expected_sha256)
}

fn current_unix_timestamp() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time before unix epoch")?
        .as_secs())
}

fn read_last_sync_time(root: &Path) -> Option<u64> {
    let path = root.join(LAST_SYNC_FILENAME);
    let text = fs::read_to_string(path).ok()?;
    text.trim().parse().ok()
}

fn write_last_sync_time(root: &Path) -> Result<()> {
    let path = root.join(LAST_SYNC_FILENAME);
    fs::write(&path, current_unix_timestamp()?.to_string())
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}
