use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use semver::Version;
use serde::{Deserialize, Serialize};

const MANIFEST_FORMAT: u32 = 1;
const MANIFEST_LIMIT: u64 = 1024 * 1024;
const PAK_LIMIT: u64 = 512 * 1024 * 1024;
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const UPDATE_THREAD_STACK: usize = 512 * 1024;
const USER_AGENT: &str = concat!("kaplay-native/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSettings {
    pub url: String,
    #[serde(default)]
    pub public_key: Option<String>,
    #[serde(default)]
    pub public_keys: Vec<String>,
}

impl UpdateSettings {
    fn keys(&self) -> Vec<&str> {
        let mut keys: Vec<&str> = self.public_keys.iter().map(String::as_str).collect();
        if let Some(key) = self.public_key.as_deref() {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        keys
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    format: u32,
    game_id: String,
    content: ManifestContent,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestContent {
    version: String,
    #[serde(default)]
    min_host: Option<String>,
    url: String,
    sha256: String,
    size: u64,
    #[serde(default)]
    rollout: Option<u8>,
    #[serde(default)]
    required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub version: String,
    pub file: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckState {
    #[serde(default)]
    last_check: u64,
    #[serde(default)]
    etag: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    current: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    required: bool,
    progress: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug)]
pub struct UpdateState {
    snapshot: Mutex<Snapshot>,
    progress: AtomicU8,
    restart: AtomicBool,
}

impl UpdateState {
    pub fn new(current: Option<String>) -> Self {
        UpdateState {
            snapshot: Mutex::new(Snapshot {
                status: "idle",
                current,
                version: None,
                required: false,
                progress: 0,
                error: None,
            }),
            progress: AtomicU8::new(0),
            restart: AtomicBool::new(false),
        }
    }

    fn set(
        &self,
        status: &'static str,
        version: Option<String>,
        required: bool,
        error: Option<String>,
    ) {
        if let Ok(mut snapshot) = self.snapshot.lock() {
            snapshot.status = status;
            snapshot.version = version;
            snapshot.required = required;
            snapshot.error = error;
        }
    }

    pub fn set_checking(&self) {
        self.progress.store(0, Ordering::Relaxed);
        self.set("checking", None, false, None);
    }

    pub fn set_up_to_date(&self, version: Option<String>) {
        self.progress.store(0, Ordering::Relaxed);
        self.set("up-to-date", version, false, None);
    }

    pub fn set_downloading(&self, version: &str, required: bool) {
        self.progress.store(0, Ordering::Relaxed);
        self.set("downloading", Some(version.to_string()), required, None);
    }

    pub fn set_progress(&self, downloaded: u64, total: u64) {
        let progress = downloaded
            .saturating_mul(100)
            .checked_div(total)
            .unwrap_or(0)
            .min(100) as u8;
        self.progress.store(progress, Ordering::Relaxed);
    }

    pub fn set_ready(&self, version: &str, required: bool) {
        self.progress.store(100, Ordering::Relaxed);
        self.set("ready", Some(version.to_string()), required, None);
    }

    pub fn set_error(&self, message: &str) {
        self.progress.store(0, Ordering::Relaxed);
        self.set("error", None, false, Some(message.to_string()));
    }

    pub fn snapshot_json(&self) -> String {
        let mut snapshot = match self.snapshot.lock() {
            Ok(snapshot) => snapshot.clone(),
            Err(_) => Snapshot {
                status: "error",
                current: None,
                version: None,
                required: false,
                progress: 0,
                error: Some("update state is unavailable".to_string()),
            },
        };
        snapshot.progress = self.progress.load(Ordering::Relaxed);
        serde_json::to_string(&snapshot).unwrap_or_else(|_| {
            "{\"status\":\"error\",\"required\":false,\"progress\":0}".to_string()
        })
    }

    pub fn request_restart(&self) -> bool {
        let ready = self
            .snapshot
            .lock()
            .map(|snapshot| snapshot.status == "ready")
            .unwrap_or(false);
        if ready {
            self.restart.store(true, Ordering::SeqCst);
        }
        ready
    }

    pub fn restart_requested(&self) -> bool {
        self.restart.load(Ordering::SeqCst)
    }
}

pub fn updates_dir(save_dir: &Path) -> PathBuf {
    save_dir.join("updates")
}

pub fn pak_path(save_dir: &Path, installed: &Installed) -> PathBuf {
    updates_dir(save_dir).join(&installed.file)
}

pub fn safe_component(value: &str) -> Option<&str> {
    if value.is_empty() || value.len() > 96 || value == "." || value == ".." {
        return None;
    }
    let valid = value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '+'));
    valid.then_some(value)
}

pub fn installed(save_dir: &Path) -> Option<Installed> {
    let dir = updates_dir(save_dir);
    let text = fs::read_to_string(dir.join("current.json")).ok()?;
    let state: Installed = serde_json::from_str(&text).ok()?;
    if safe_component(&state.file).is_none() {
        log::warn!("ignoring installed content with an unsafe file name");
        return None;
    }
    let metadata = match fs::metadata(dir.join(&state.file)) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => {
            log::warn!("installed content {} is not a file", state.version);
            return None;
        }
        Err(error) => {
            log::warn!("installed content {} is missing: {error}", state.version);
            return None;
        }
    };
    if metadata.len() != state.size {
        log::warn!(
            "installed content {} has {} bytes, expected {}",
            state.version,
            metadata.len(),
            state.size
        );
        return None;
    }
    Some(state)
}

pub fn preferred(installed: Option<Installed>, embedded: Option<&str>) -> Option<Installed> {
    let installed = installed?;
    let Some(embedded) = embedded else {
        return Some(installed);
    };
    match (Version::parse(&installed.version), Version::parse(embedded)) {
        (Ok(installed_version), Ok(embedded_version)) if installed_version <= embedded_version => {
            log::info!(
                "ignoring installed content {} because the packaged content {embedded} is at least as new",
                installed.version
            );
            None
        }
        (Ok(_), Ok(_)) => Some(installed),
        _ => {
            log::warn!(
                "could not compare installed content {} with packaged content {embedded}; using the packaged content",
                installed.version
            );
            None
        }
    }
}

pub fn reset(save_dir: &Path) {
    let dir = updates_dir(save_dir);
    match fs::remove_dir_all(&dir) {
        Ok(()) => log::info!("removed installed content at {}", dir.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => log::warn!("failed to remove {}: {error}", dir.display()),
    }
}

pub fn spawn_check(
    settings: UpdateSettings,
    save_dir: PathBuf,
    game_id: String,
    current_version: String,
    state: Arc<UpdateState>,
) {
    let Some(game_id) = safe_component(&game_id).map(str::to_owned) else {
        log::warn!("ignoring update settings without a valid game id");
        state.set_error("invalid game id");
        return;
    };
    let host_version = env!("CARGO_PKG_VERSION").to_string();
    let thread_state = Arc::clone(&state);
    let spawned = std::thread::Builder::new()
        .name("kaplay-native-update".to_string())
        .stack_size(UPDATE_THREAD_STACK)
        .spawn(move || {
            let dir = updates_dir(&save_dir);
            if let Err(error) = fs::create_dir_all(&dir) {
                let message = format!("failed to prepare {}: {error}", dir.display());
                log::warn!("{message}");
                thread_state.set_error(&message);
                return;
            }
            let mut check_state = read_check_state(&dir);
            if now_seconds().saturating_sub(check_state.last_check) < CHECK_INTERVAL.as_secs() {
                log::debug!("content update check is not due yet");
                return;
            }
            check_state.last_check = now_seconds();
            write_check_state(&dir, &check_state);
            thread_state.set_checking();
            match check(
                &settings,
                &save_dir,
                &game_id,
                &current_version,
                &host_version,
                check_state.etag.as_deref(),
                &thread_state,
            ) {
                Ok((version, etag)) => {
                    if etag.is_some() {
                        check_state.etag = etag;
                    }
                    write_check_state(&dir, &check_state);
                    match version {
                        Some(version) => log::info!(
                            "content update {version} installed; it will be used on the next launch"
                        ),
                        None => log::debug!("no content update available"),
                    }
                }
                Err(error) => {
                    thread_state.set_error(&error);
                    log::warn!("content update check failed: {error}");
                }
            }
        });
    if let Err(error) = spawned {
        let message = format!("failed to start the content update check: {error}");
        log::warn!("{message}");
        state.set_error(&message);
    }
}

fn check(
    settings: &UpdateSettings,
    save_dir: &Path,
    game_id: &str,
    current_version: &str,
    host_version: &str,
    etag: Option<&str>,
    state: &UpdateState,
) -> Result<(Option<String>, Option<String>), String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let agent = agent();
    let ManifestResponse {
        not_modified,
        body,
        etag: response_etag,
    } = fetch_manifest(&agent, &settings.url, etag)?;
    if not_modified {
        state.set_up_to_date(None);
        return Ok((None, None));
    }
    let signature_bytes = fetch(
        &agent,
        &signature_url(&settings.url),
        MANIFEST_LIMIT,
        "text/plain",
    )?;
    let signature = std::str::from_utf8(&signature_bytes)
        .map_err(|_| "the manifest signature is not valid utf-8".to_string())?;
    let keys = settings.keys();
    let content = verify_manifest(&body, signature, &keys, game_id)?;
    let current = Version::parse(current_version).unwrap_or_else(|_| Version::new(0, 0, 0));
    let next = Version::parse(&content.version)
        .map_err(|error| format!("invalid content version {}: {error}", content.version))?;
    if next <= current {
        state.set_up_to_date(Some(content.version));
        return Ok((None, response_etag));
    }
    let host = Version::parse(host_version)
        .map_err(|error| format!("invalid host version {host_version}: {error}"))?;
    if !host_supports(&content, &host)? {
        log::info!(
            "content {} needs a newer host; keeping the current content",
            content.version
        );
        state.set_up_to_date(Some(content.version));
        return Ok((None, response_etag));
    }
    if !rollout_allows(&content, save_dir, game_id) {
        log::info!(
            "content {} is not rolled out to this install yet",
            content.version
        );
        state.set_up_to_date(Some(content.version));
        return Ok((None, response_etag));
    }
    state.set_downloading(&content.version, content.required);
    let dir = updates_dir(save_dir);
    fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
    let (temp, size, hash) = download(&agent, &content.url, content.size, &dir, state)?;
    if size != content.size || hash != content.sha256.to_ascii_lowercase() {
        let _ = fs::remove_file(&temp);
        return Err("downloaded content does not match the manifest".to_string());
    }
    if let Err(error) = install_download(save_dir, &content, &temp) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    state.set_ready(&content.version, content.required);
    Ok((Some(content.version), response_etag))
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(300)))
        .build()
        .new_agent()
}

struct ManifestResponse {
    not_modified: bool,
    body: Vec<u8>,
    etag: Option<String>,
}

fn fetch_manifest(
    agent: &ureq::Agent,
    url: &str,
    etag: Option<&str>,
) -> Result<ManifestResponse, String> {
    let mut request = agent
        .get(url)
        .header("Accept", "application/json")
        .header("User-Agent", USER_AGENT);
    if let Some(etag) = etag {
        request = request.header("If-None-Match", etag);
    }
    let response = request
        .call()
        .map_err(|error| format!("GET {url}: {error}"))?;
    let status = response.status();
    if status.as_u16() == 304 {
        return Ok(ManifestResponse {
            not_modified: true,
            body: Vec::new(),
            etag: None,
        });
    }
    if !status.is_success() {
        return Err(format!("GET {url}: HTTP {}", status.as_u16()));
    }
    let etag = response
        .headers()
        .get("etag")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let mut body = response.into_body();
    let bytes = body
        .with_config()
        .limit(MANIFEST_LIMIT)
        .read_to_vec()
        .map_err(|error| format!("GET {url}: {error}"))?;
    Ok(ManifestResponse {
        not_modified: false,
        body: bytes,
        etag,
    })
}

fn fetch(agent: &ureq::Agent, url: &str, limit: u64, accept: &str) -> Result<Vec<u8>, String> {
    let response = agent
        .get(url)
        .header("Accept", accept)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|error| format!("GET {url}: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("GET {url}: HTTP {}", status.as_u16()));
    }
    let mut body = response.into_body();
    body.with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|error| format!("GET {url}: {error}"))
}

fn download(
    agent: &ureq::Agent,
    url: &str,
    expected: u64,
    dir: &Path,
    state: &UpdateState,
) -> Result<(PathBuf, u64, String), String> {
    let temp = dir.join(format!("download-{}.tmp", std::process::id()));
    let result = (|| {
        let response = agent
            .get(url)
            .header("Accept", "application/octet-stream")
            .header("User-Agent", USER_AGENT)
            .call()
            .map_err(|error| format!("GET {url}: {error}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("GET {url}: HTTP {}", status.as_u16()));
        }
        let mut reader = response.into_body().into_reader().take(expected + 1);
        let mut file = File::create(&temp)
            .map_err(|error| format!("failed to write {}: {error}", temp.display()))?;
        let mut hasher = ring::digest::Context::new(&ring::digest::SHA256);
        let mut chunk = [0u8; 64 * 1024];
        let mut size = 0u64;
        loop {
            let read = reader
                .read(&mut chunk)
                .map_err(|error| format!("GET {url}: {error}"))?;
            if read == 0 {
                break;
            }
            file.write_all(&chunk[..read])
                .map_err(|error| format!("failed to write {}: {error}", temp.display()))?;
            hasher.update(&chunk[..read]);
            size += read as u64;
            state.set_progress(size, expected);
        }
        file.sync_all()
            .map_err(|error| format!("failed to write {}: {error}", temp.display()))?;
        let hash = hasher
            .finish()
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok((size, hash))
    })();
    match result {
        Ok((size, hash)) => Ok((temp, size, hash)),
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(error)
        }
    }
}

fn signature_url(url: &str) -> String {
    match url.find('?') {
        Some(index) => format!("{}.sig{}", &url[..index], &url[index..]),
        None => format!("{url}.sig"),
    }
}

fn verify_manifest(
    bytes: &[u8],
    signature: &str,
    keys: &[&str],
    game_id: &str,
) -> Result<ManifestContent, String> {
    let signature = base64::engine::general_purpose::STANDARD
        .decode(signature.trim())
        .map_err(|error| format!("invalid manifest signature: {error}"))?;
    let mut verified = false;
    for key in keys {
        let Ok(key) = base64::engine::general_purpose::STANDARD.decode(key.trim()) else {
            continue;
        };
        if key.len() != 32 {
            continue;
        }
        if ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &key)
            .verify(bytes, &signature)
            .is_ok()
        {
            verified = true;
            break;
        }
    }
    if !verified {
        return Err("manifest signature does not match a trusted key".to_string());
    }
    let manifest: Manifest =
        serde_json::from_slice(bytes).map_err(|error| format!("invalid manifest: {error}"))?;
    if manifest.format != MANIFEST_FORMAT {
        return Err(format!(
            "unsupported manifest format {}, expected {MANIFEST_FORMAT}",
            manifest.format
        ));
    }
    if manifest.game_id != game_id {
        return Err(format!(
            "manifest is for game '{}', expected '{game_id}'",
            manifest.game_id
        ));
    }
    let content: ManifestContent = manifest.content;
    if !is_sha256(&content.sha256) {
        return Err("manifest sha256 is not a hex digest".to_string());
    }
    if content.size == 0 || content.size > PAK_LIMIT {
        return Err(format!(
            "manifest declares an unsupported size: {}",
            content.size
        ));
    }
    if !content.url.starts_with("https://") && !content.url.starts_with("http://") {
        return Err("manifest content url must be absolute http(s)".to_string());
    }
    Version::parse(&content.version)
        .map_err(|error| format!("invalid content version {}: {error}", content.version))?;
    if let Some(min_host) = &content.min_host {
        Version::parse(min_host).map_err(|error| format!("invalid minHost {min_host}: {error}"))?;
    }
    if content.rollout.is_some_and(|rollout| rollout > 100) {
        return Err("manifest rollout must be between 0 and 100".to_string());
    }
    Ok(content)
}

fn host_supports(content: &ManifestContent, host: &Version) -> Result<bool, String> {
    let Some(min_host) = &content.min_host else {
        return Ok(true);
    };
    let min_host =
        Version::parse(min_host).map_err(|error| format!("invalid minHost {min_host}: {error}"))?;
    Ok(*host >= min_host)
}

fn rollout_allows(content: &ManifestContent, save_dir: &Path, game_id: &str) -> bool {
    let rollout = content.rollout.unwrap_or(100);
    if rollout >= 100 {
        return true;
    }
    let bucket = install_bucket(save_dir, game_id).unwrap_or(0);
    bucket < rollout
}

fn install_bucket(save_dir: &Path, game_id: &str) -> Option<u8> {
    let id = install_id(save_dir)?;
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    context.update(id.as_bytes());
    context.update(b":");
    context.update(game_id.as_bytes());
    let digest = context.finish();
    Some((u16::from_be_bytes([digest.as_ref()[0], digest.as_ref()[1]]) % 100) as u8)
}

fn install_id(save_dir: &Path) -> Option<String> {
    let path = save_dir.join("install-id");
    if let Ok(text) = fs::read_to_string(&path) {
        let text = text.trim();
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    fs::create_dir_all(save_dir).ok()?;
    use ring::rand::SecureRandom;
    let rng = ring::rand::SystemRandom::new();
    let mut bytes = [0u8; 16];
    rng.fill(&mut bytes).ok()?;
    let id: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    write_atomic(&path, id.as_bytes()).ok()?;
    Some(id)
}

#[cfg(test)]
fn install(save_dir: &Path, content: &ManifestContent, bytes: &[u8]) -> Result<(), String> {
    let dir = updates_dir(save_dir);
    fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
    let file = content_file(content)?;
    write_atomic(&dir.join(&file), bytes)?;
    write_state(&dir, content, file)
}

fn install_download(save_dir: &Path, content: &ManifestContent, temp: &Path) -> Result<(), String> {
    let file = content_file(content)?;
    let dir = updates_dir(save_dir);
    fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
    let target = dir.join(&file);
    replace(temp, &target)
        .map_err(|error| format!("failed to install {}: {error}", target.display()))?;
    write_state(&dir, content, file)
}

fn content_file(content: &ManifestContent) -> Result<String, String> {
    let Some(version) = safe_component(&content.version) else {
        return Err(format!("unsafe content version {}", content.version));
    };
    Ok(format!("content-{version}.pak"))
}

fn write_state(dir: &Path, content: &ManifestContent, file: String) -> Result<(), String> {
    let state = Installed {
        version: content.version.clone(),
        file,
        sha256: content.sha256.to_ascii_lowercase(),
        size: content.size,
    };
    let json = serde_json::to_vec_pretty(&state)
        .map_err(|error| format!("invalid update state: {error}"))?;
    write_atomic(&dir.join("current.json"), &json)?;
    prune(dir, &state.file);
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    {
        let mut file = File::create(&tmp)
            .map_err(|error| format!("failed to write {}: {error}", tmp.display()))?;
        file.write_all(bytes)
            .map_err(|error| format!("failed to write {}: {error}", tmp.display()))?;
        file.sync_all()
            .map_err(|error| format!("failed to write {}: {error}", tmp.display()))?;
    }
    replace(&tmp, path).map_err(|error| {
        let _ = fs::remove_file(&tmp);
        format!("failed to install {}: {error}", path.display())
    })
}

#[cfg(not(windows))]
fn replace(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)
}

#[cfg(windows)]
fn replace(from: &Path, to: &Path) -> std::io::Result<()> {
    if to.exists() {
        fs::remove_file(to)?;
    }
    fs::rename(from, to)
}

fn prune(dir: &Path, current: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut previous = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name == current || !name.starts_with("content-") || !name.ends_with(".pak") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH);
        previous.push((modified, name.to_string()));
    }
    previous.sort_by_key(|(modified, _)| *modified);
    for (_, name) in previous.into_iter().rev().skip(1) {
        let _ = fs::remove_file(dir.join(name));
    }
}

#[cfg(test)]
fn sha256_hex(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn read_check_state(dir: &Path) -> CheckState {
    fs::read_to_string(dir.join("state.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_check_state(dir: &Path, state: &CheckState) {
    let Ok(json) = serde_json::to_vec(state) else {
        return;
    };
    let _ = write_atomic(&dir.join("state.json"), &json);
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    use ring::signature::KeyPair;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("kap-update-{name}-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn keypair() -> (String, ring::signature::Ed25519KeyPair) {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let key = base64::engine::general_purpose::STANDARD.encode(pair.public_key().as_ref());
        (key, pair)
    }

    fn manifest(url: &str, version: &str, bytes: &[u8], game_id: &str) -> Vec<u8> {
        serde_json::json!({
            "format": 1,
            "gameId": game_id,
            "content": {
                "version": version,
                "url": url,
                "sha256": sha256_hex(bytes),
                "size": bytes.len(),
            }
        })
        .to_string()
        .into_bytes()
    }

    fn signature(pair: &ring::signature::Ed25519KeyPair, bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(pair.sign(bytes).as_ref())
    }

    fn content(version: &str, bytes: &[u8]) -> ManifestContent {
        ManifestContent {
            version: version.to_string(),
            min_host: None,
            url: "https://cdn.example.com/game.pak".to_string(),
            sha256: sha256_hex(bytes),
            size: bytes.len() as u64,
            rollout: None,
            required: false,
        }
    }

    #[test]
    fn verifies_a_signed_manifest() {
        let (key, pair) = keypair();
        let bytes = manifest(
            "https://cdn.example.com/game.pak",
            "1.2.0",
            b"pak",
            "bridge",
        );
        let content =
            verify_manifest(&bytes, &signature(&pair, &bytes), &[key.as_str()], "bridge").unwrap();
        assert_eq!(content.version, "1.2.0");
        assert_eq!(content.size, 3);
        assert!(!content.required);
    }

    #[test]
    fn verifies_with_any_trusted_key() {
        let (first, _) = keypair();
        let (second, pair) = keypair();
        let bytes = manifest(
            "https://cdn.example.com/game.pak",
            "1.2.0",
            b"pak",
            "bridge",
        );
        let result = verify_manifest(
            &bytes,
            &signature(&pair, &bytes),
            &[first.as_str(), second.as_str()],
            "bridge",
        );
        assert!(result.is_ok());
    }

    #[test]
    fn rejects_a_tampered_manifest() {
        let (key, pair) = keypair();
        let mut bytes = manifest(
            "https://cdn.example.com/game.pak",
            "1.2.0",
            b"pak",
            "bridge",
        );
        let sig = signature(&pair, &bytes);
        bytes[0] = b'[';
        let error = verify_manifest(&bytes, &sig, &[key.as_str()], "bridge").unwrap_err();
        assert!(error.contains("signature"));
    }

    #[test]
    fn rejects_another_game() {
        let (key, pair) = keypair();
        let bytes = manifest(
            "https://cdn.example.com/game.pak",
            "1.2.0",
            b"pak",
            "bridge",
        );
        let error = verify_manifest(
            &bytes,
            &signature(&pair, &bytes),
            &[key.as_str()],
            "other-game",
        )
        .unwrap_err();
        assert!(error.contains("other-game"));
    }

    #[test]
    fn reads_rollout_and_required() {
        let (key, pair) = keypair();
        let bytes = serde_json::json!({
            "format": 1,
            "gameId": "bridge",
            "content": {
                "version": "1.2.0",
                "url": "https://cdn.example.com/game.pak",
                "sha256": sha256_hex(b"pak"),
                "size": 3,
                "rollout": 25,
                "required": true,
            }
        })
        .to_string()
        .into_bytes();
        let content =
            verify_manifest(&bytes, &signature(&pair, &bytes), &[key.as_str()], "bridge").unwrap();
        assert_eq!(content.rollout, Some(25));
        assert!(content.required);
    }

    #[test]
    fn checks_the_minimum_host() {
        let (key, pair) = keypair();
        let bytes = serde_json::json!({
            "format": 1,
            "gameId": "bridge",
            "content": {
                "version": "1.2.0",
                "minHost": "0.2.0",
                "url": "https://cdn.example.com/game.pak",
                "sha256": sha256_hex(b"pak"),
                "size": 3,
            }
        })
        .to_string()
        .into_bytes();
        let content =
            verify_manifest(&bytes, &signature(&pair, &bytes), &[key.as_str()], "bridge").unwrap();
        assert!(!host_supports(&content, &Version::new(0, 1, 0)).unwrap());
        assert!(host_supports(&content, &Version::new(0, 2, 0)).unwrap());
    }

    #[test]
    fn installs_and_reads_content() {
        let save_dir = temp_dir("install");
        let bytes = b"pak bytes";
        install(&save_dir, &content("1.2.0", bytes), bytes).unwrap();
        let state = installed(&save_dir).unwrap();
        assert_eq!(state.version, "1.2.0");
        assert_eq!(fs::read(pak_path(&save_dir, &state)).unwrap(), bytes);
        fs::remove_dir_all(save_dir).ok();
    }

    #[test]
    fn ignores_content_with_the_wrong_size() {
        let save_dir = temp_dir("corrupt");
        let bytes = b"pak bytes";
        install(&save_dir, &content("1.2.0", bytes), bytes).unwrap();
        let state = installed(&save_dir).unwrap();
        fs::write(pak_path(&save_dir, &state), b"damaged").unwrap();
        assert!(installed(&save_dir).is_none());
        fs::remove_dir_all(save_dir).ok();
    }

    #[test]
    fn packaged_content_wins_when_it_is_not_older() {
        let save_dir = temp_dir("preferred");
        let bytes = b"pak bytes";
        install(&save_dir, &content("1.2.0", bytes), bytes).unwrap();
        assert!(preferred(installed(&save_dir), Some("1.2.0")).is_none());
        assert!(preferred(installed(&save_dir), Some("1.3.0")).is_none());
        assert!(preferred(installed(&save_dir), Some("1.1.0")).is_some());
        fs::remove_dir_all(save_dir).ok();
    }

    #[test]
    fn exposes_update_status_and_restart() {
        let state = UpdateState::new(Some("1.0.0".to_string()));
        let snapshot: serde_json::Value = serde_json::from_str(&state.snapshot_json()).unwrap();
        assert_eq!(snapshot["status"], "idle");
        assert_eq!(snapshot["current"], "1.0.0");
        assert!(!state.request_restart());

        state.set_ready("1.1.0", true);
        let snapshot: serde_json::Value = serde_json::from_str(&state.snapshot_json()).unwrap();
        assert_eq!(snapshot["status"], "ready");
        assert_eq!(snapshot["version"], "1.1.0");
        assert_eq!(snapshot["required"], true);
        assert_eq!(snapshot["progress"], 100);
        assert!(state.request_restart());
        assert!(state.restart_requested());
    }

    #[test]
    fn rollout_uses_a_stable_bucket() {
        let save_dir = temp_dir("rollout");
        fs::write(
            save_dir.join("install-id"),
            "00112233445566778899aabbccddeeff",
        )
        .unwrap();
        let bucket = install_bucket(&save_dir, "bridge").unwrap();
        assert!(bucket < 100);
        let mut item = content("1.2.0", b"pak");
        item.rollout = Some(bucket + 1);
        assert!(rollout_allows(&item, &save_dir, "bridge"));
        item.rollout = Some(bucket);
        assert!(!rollout_allows(&item, &save_dir, "bridge"));
        item.rollout = Some(100);
        assert!(rollout_allows(&item, &save_dir, "bridge"));
        item.rollout = Some(0);
        assert!(!rollout_allows(&item, &save_dir, "bridge"));
        fs::remove_dir_all(save_dir).ok();
    }

    #[test]
    fn downloads_and_installs_a_signed_update() {
        let pak = b"fresh pak".to_vec();
        let (key, pair) = keypair();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let manifest = manifest(&format!("{base}/game.pak"), "1.2.0", &pak, "bridge");
        let sig = signature(&pair, &manifest);
        let served = pak.clone();
        let server = std::thread::spawn(move || {
            for _ in 0..4 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    headers.push_str(&line);
                }
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let not_modified = path == "/manifest.json"
                    && headers
                        .to_ascii_lowercase()
                        .contains("if-none-match: \"v1\"");
                if not_modified {
                    stream
                        .write_all(
                            b"HTTP/1.1 304 Not Modified\r\nETag: \"v1\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .unwrap();
                    continue;
                }
                let body = match path {
                    "/manifest.json" => manifest.clone(),
                    "/manifest.json.sig" => sig.clone().into_bytes(),
                    "/game.pak" => served.clone(),
                    _ => Vec::new(),
                };
                let status = if body.is_empty() {
                    "404 Not Found"
                } else {
                    "200 OK"
                };
                let etag = if path == "/manifest.json" {
                    "ETag: \"v1\"\r\n"
                } else {
                    ""
                };
                let header = format!(
                    "HTTP/1.1 {status}\r\n{etag}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(header.as_bytes()).unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        let save_dir = temp_dir("http");
        let settings = UpdateSettings {
            url: format!("{base}/manifest.json"),
            public_key: Some(key),
            public_keys: Vec::new(),
        };
        let state = UpdateState::new(Some("1.0.0".to_string()));
        let (version, etag) = check(
            &settings, &save_dir, "bridge", "1.0.0", "0.1.0", None, &state,
        )
        .unwrap();
        assert_eq!(version.as_deref(), Some("1.2.0"));
        assert_eq!(etag.as_deref(), Some("\"v1\""));
        let (version, _) = check(
            &settings,
            &save_dir,
            "bridge",
            "1.0.0",
            "0.1.0",
            etag.as_deref(),
            &state,
        )
        .unwrap();
        assert!(version.is_none());
        server.join().unwrap();
        let snapshot: serde_json::Value = serde_json::from_str(&state.snapshot_json()).unwrap();
        assert_eq!(snapshot["status"], "up-to-date");
        let installed = installed(&save_dir).unwrap();
        assert_eq!(fs::read(pak_path(&save_dir, &installed)).unwrap(), pak);
        fs::remove_dir_all(save_dir).ok();
    }
}
