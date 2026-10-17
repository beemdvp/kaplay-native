use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde::Deserialize;

use crate::updates::UpdateSettings;

pub const MAGIC: &[u8; 8] = b"KNATIVEP";
pub const VERSION: u32 = 1;
pub const HEADER_LEN: u64 = 24;

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PackConfig {
    pub js_engine: Option<String>,
    pub title: Option<String>,
    pub window: Option<String>,
    pub network: Option<bool>,
    pub url: Option<String>,
    pub game_id: Option<String>,
    pub content_version: Option<String>,
    pub update: Option<UpdateSettings>,
}

pub struct Embedded {
    pub pak: Vec<u8>,
    pub config: PackConfig,
}

pub fn read(path: &Path) -> Option<Embedded> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if len < HEADER_LEN {
        return None;
    }
    file.seek(SeekFrom::End(-(HEADER_LEN as i64))).ok()?;
    let mut header = [0u8; HEADER_LEN as usize];
    file.read_exact(&mut header).ok()?;
    if &header[0..8] != MAGIC {
        return None;
    }
    let version = u32::from_le_bytes(header[8..12].try_into().ok()?);
    if version != VERSION {
        return None;
    }
    let pak_len = u64::from_le_bytes(header[12..20].try_into().ok()?);
    let config_len = u64::from(u32::from_le_bytes(header[20..24].try_into().ok()?));
    let payload = pak_len.checked_add(config_len)?.checked_add(HEADER_LEN)?;
    if payload > len {
        return None;
    }
    let config_start = len - HEADER_LEN - config_len;
    let pak_start = config_start - pak_len;
    let config = if config_len == 0 {
        PackConfig::default()
    } else {
        let mut bytes = vec![0u8; config_len as usize];
        file.seek(SeekFrom::Start(config_start)).ok()?;
        file.read_exact(&mut bytes).ok()?;
        serde_json::from_slice(&bytes).unwrap_or_default()
    };
    let mut pak = vec![0u8; pak_len as usize];
    file.seek(SeekFrom::Start(pak_start)).ok()?;
    file.read_exact(&mut pak).ok()?;
    Some(Embedded { pak, config })
}

pub fn read_current() -> Option<Embedded> {
    read(&std::env::current_exe().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_embedded(path: &Path, pak: &[u8], config: &str) {
        let mut bytes = b"host".to_vec();
        bytes.extend_from_slice(pak);
        bytes.extend_from_slice(config.as_bytes());
        let mut header = [0u8; HEADER_LEN as usize];
        header[0..8].copy_from_slice(MAGIC);
        header[8..12].copy_from_slice(&VERSION.to_le_bytes());
        header[12..20].copy_from_slice(&(pak.len() as u64).to_le_bytes());
        header[20..24].copy_from_slice(&(config.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&header);
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn reads_embedded_pak_and_config() {
        let path = std::env::temp_dir().join(format!("kap-embed-{}", std::process::id()));
        write_embedded(
            &path,
            b"pak bytes",
            r#"{"jsEngine":"v8","title":"Bridge","window":"320x240","network":true}"#,
        );
        let embedded = read(&path).unwrap();
        assert_eq!(embedded.pak, b"pak bytes");
        assert_eq!(embedded.config.js_engine.as_deref(), Some("v8"));
        assert_eq!(embedded.config.title.as_deref(), Some("Bridge"));
        assert_eq!(embedded.config.window.as_deref(), Some("320x240"));
        assert_eq!(embedded.config.network, Some(true));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn reads_update_config() {
        let path = std::env::temp_dir().join(format!("kap-embed-update-{}", std::process::id()));
        write_embedded(
            &path,
            b"pak bytes",
            r#"{"gameId":"bridge","contentVersion":"1.2.0","update":{"url":"https://cdn.example.com/manifest.json","publicKey":"a2V5"}}"#,
        );
        let embedded = read(&path).unwrap();
        assert_eq!(embedded.config.game_id.as_deref(), Some("bridge"));
        assert_eq!(embedded.config.content_version.as_deref(), Some("1.2.0"));
        let update = embedded.config.update.unwrap();
        assert_eq!(update.url, "https://cdn.example.com/manifest.json");
        assert_eq!(update.public_key.as_deref(), Some("a2V5"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn defaults_when_config_is_empty() {
        let path = std::env::temp_dir().join(format!("kap-embed-empty-{}", std::process::id()));
        write_embedded(&path, b"pak bytes", "");
        let embedded = read(&path).unwrap();
        assert_eq!(embedded.pak, b"pak bytes");
        assert!(embedded.config.js_engine.is_none());
        assert!(embedded.config.title.is_none());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn ignores_files_without_a_trailer() {
        let path = std::env::temp_dir().join(format!("kap-embed-plain-{}", std::process::id()));
        std::fs::write(&path, b"just a host binary").unwrap();
        assert!(read(&path).is_none());
        std::fs::write(&path, b"short").unwrap();
        assert!(read(&path).is_none());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn ignores_a_truncated_payload() {
        let path = std::env::temp_dir().join(format!("kap-embed-trunc-{}", std::process::id()));
        write_embedded(&path, b"pak bytes", "");
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.truncate(bytes.len() - 2);
        std::fs::write(&path, bytes).unwrap();
        assert!(read(&path).is_none());
        std::fs::remove_file(&path).ok();
    }
}
