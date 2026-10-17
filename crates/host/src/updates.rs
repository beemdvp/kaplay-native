use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use semver::Version;
use serde::{Deserialize, Serialize};

const MANIFEST_FORMAT: u32 = 1;
const MANIFEST_LIMIT: u64 = 1024 * 1024;
const PAK_LIMIT: u64 = 512 * 1024 * 1024;
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const UPDATE_THREAD_STACK: usize = 512 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSettings {
    pub url: String,
    pub public_key: String,
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
    last_check: u64,
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
) {
    let Some(game_id) = safe_component(&game_id).map(str::to_owned) else {
        log::warn!("ignoring update settings without a valid game id");
        return;
    };
    let host_version = env!("CARGO_PKG_VERSION").to_string();
    let spawned = std::thread::Builder::new()
        .name("kaplay-native-update".to_string())
        .stack_size(UPDATE_THREAD_STACK)
        .spawn(move || {
            let dir = updates_dir(&save_dir);
            if let Err(error) = fs::create_dir_all(&dir) {
                log::warn!("failed to prepare {}: {error}", dir.display());
                return;
            }
            if !check_due(&dir) {
                log::debug!("content update check is not due yet");
                return;
            }
            write_check_time(&dir);
            match check(
                &settings,
                &save_dir,
                &game_id,
                &current_version,
                &host_version,
            ) {
                Ok(Some(version)) => log::info!(
                    "content update {version} installed; it will be used on the next launch"
                ),
                Ok(None) => log::debug!("no content update available"),
                Err(error) => log::warn!("content update check failed: {error}"),
            }
        });
    if let Err(error) = spawned {
        log::warn!("failed to start the content update check: {error}");
    }
}

fn check(
    settings: &UpdateSettings,
    save_dir: &Path,
    game_id: &str,
    current_version: &str,
    host_version: &str,
) -> Result<Option<String>, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let agent = agent();
    let manifest_bytes = fetch(&agent, &settings.url, MANIFEST_LIMIT, "application/json")?;
    let signature_bytes = fetch(
        &agent,
        &signature_url(&settings.url),
        MANIFEST_LIMIT,
        "text/plain",
    )?;
    let signature = std::str::from_utf8(&signature_bytes)
        .map_err(|_| "the manifest signature is not valid utf-8".to_string())?;
    let host = Version::parse(host_version)
        .map_err(|error| format!("invalid host version {host_version}: {error}"))?;
    let content = verify_manifest(
        &manifest_bytes,
        signature,
        &settings.public_key,
        game_id,
        &host,
    )?;
    let current = Version::parse(current_version).unwrap_or_else(|_| Version::new(0, 0, 0));
    let next = Version::parse(&content.version)
        .map_err(|error| format!("invalid content version {}: {error}", content.version))?;
    if next <= current {
        return Ok(None);
    }
    let dir = updates_dir(save_dir);
    fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
    let (temp, size, hash) = download(&agent, &content.url, content.size, &dir)?;
    if size != content.size || hash != content.sha256.to_ascii_lowercase() {
        let _ = fs::remove_file(&temp);
        return Err("downloaded content does not match the manifest".to_string());
    }
    if let Err(error) = install_download(save_dir, &content, &temp) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    Ok(Some(content.version))
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(300)))
        .build()
        .new_agent()
}

fn fetch(agent: &ureq::Agent, url: &str, limit: u64, accept: &str) -> Result<Vec<u8>, String> {
    let response = agent
        .get(url)
        .header("Accept", accept)
        .header(
            "User-Agent",
            concat!("kaplay-native/", env!("CARGO_PKG_VERSION")),
        )
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
) -> Result<(PathBuf, u64, String), String> {
    let temp = dir.join(format!("download-{}.tmp", std::process::id()));
    let result = (|| {
        let response = agent
            .get(url)
            .header("Accept", "application/octet-stream")
            .header(
                "User-Agent",
                concat!("kaplay-native/", env!("CARGO_PKG_VERSION")),
            )
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
    public_key: &str,
    game_id: &str,
    host: &Version,
) -> Result<ManifestContent, String> {
    let key = base64::engine::general_purpose::STANDARD
        .decode(public_key.trim())
        .map_err(|error| format!("invalid update public key: {error}"))?;
    if key.len() != 32 {
        return Err("update public key must be 32 bytes".to_string());
    }
    let signature = base64::engine::general_purpose::STANDARD
        .decode(signature.trim())
        .map_err(|error| format!("invalid manifest signature: {error}"))?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &key)
        .verify(bytes, &signature)
        .map_err(|_| "manifest signature does not match".to_string())?;
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
        let min_host = Version::parse(min_host)
            .map_err(|error| format!("invalid minHost {min_host}: {error}"))?;
        if *host < min_host {
            return Err(format!(
                "content {} needs host {min_host}, this is {host}",
                content.version
            ));
        }
    }
    Ok(content)
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

fn check_due(dir: &Path) -> bool {
    let Ok(text) = fs::read_to_string(dir.join("state.json")) else {
        return true;
    };
    let Ok(state) = serde_json::from_str::<CheckState>(&text) else {
        return true;
    };
    now_seconds().saturating_sub(state.last_check) >= CHECK_INTERVAL.as_secs()
}

fn write_check_time(dir: &Path) {
    let state = CheckState {
        last_check: now_seconds(),
    };
    let Ok(json) = serde_json::to_vec(&state) else {
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
        let content = verify_manifest(
            &bytes,
            &signature(&pair, &bytes),
            &key,
            "bridge",
            &Version::new(0, 1, 0),
        )
        .unwrap();
        assert_eq!(content.version, "1.2.0");
        assert_eq!(content.size, 3);
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
        let error =
            verify_manifest(&bytes, &sig, &key, "bridge", &Version::new(0, 1, 0)).unwrap_err();
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
            &key,
            "other-game",
            &Version::new(0, 1, 0),
        )
        .unwrap_err();
        assert!(error.contains("other-game"));
    }

    #[test]
    fn rejects_a_newer_minimum_host() {
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
        let error = verify_manifest(
            &bytes,
            &signature(&pair, &bytes),
            &key,
            "bridge",
            &Version::new(0, 1, 0),
        )
        .unwrap_err();
        assert!(error.contains("needs host"));
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
    fn downloads_and_installs_a_signed_update() {
        let pak = b"fresh pak".to_vec();
        let (key, pair) = keypair();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let manifest = manifest(&format!("{base}/game.pak"), "1.2.0", &pak, "bridge");
        let sig = signature(&pair, &manifest);
        let served = pak.clone();
        let server = std::thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                let path = request.split_whitespace().nth(1).unwrap_or("/");
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
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(header.as_bytes()).unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        let save_dir = temp_dir("http");
        let settings = UpdateSettings {
            url: format!("{base}/manifest.json"),
            public_key: key,
        };
        let version = check(&settings, &save_dir, "bridge", "1.0.0", "0.1.0").unwrap();
        assert_eq!(version.as_deref(), Some("1.2.0"));
        server.join().unwrap();
        let state = installed(&save_dir).unwrap();
        assert_eq!(fs::read(pak_path(&save_dir, &state)).unwrap(), pak);
        fs::remove_dir_all(save_dir).ok();
    }
}
