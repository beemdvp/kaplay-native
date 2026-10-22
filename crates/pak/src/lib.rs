use std::collections::{HashMap, VecDeque};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PakError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid pak archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("entry name escapes the archive root: {0}")]
    UnsafeEntry(String),
}

const CACHE_LIMIT: usize = 64 * 1024 * 1024;

#[derive(Debug)]
struct EntryCache {
    entries: HashMap<String, Arc<[u8]>>,
    order: VecDeque<String>,
    bytes: usize,
    limit: usize,
}

impl EntryCache {
    fn new(limit: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
            limit,
        }
    }

    fn get(&self, name: &str) -> Option<Arc<[u8]>> {
        self.entries.get(name).cloned()
    }

    fn insert(&mut self, name: &str, bytes: Arc<[u8]>) {
        if bytes.len() > self.limit {
            return;
        }
        if let Some(previous) = self.entries.insert(name.to_string(), Arc::clone(&bytes)) {
            self.bytes = self.bytes.saturating_sub(previous.len());
        } else {
            self.order.push_back(name.to_string());
        }
        self.bytes += bytes.len();
        while self.bytes > self.limit {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(removed.len());
            }
        }
    }
}

#[derive(Debug)]
struct Archive {
    entries: HashMap<String, (usize, u64)>,
    zip: Mutex<zip::ZipArchive<Cursor<Arc<[u8]>>>>,
    cache: Mutex<EntryCache>,
}

impl Archive {
    fn read(&self, name: &str) -> Option<Arc<[u8]>> {
        if let Ok(cache) = self.cache.lock() {
            if let Some(bytes) = cache.get(name) {
                return Some(bytes);
            }
        }
        let bytes = self.read_uncached(name)?;
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(name, Arc::clone(&bytes));
        }
        Some(bytes)
    }

    fn read_uncached(&self, name: &str) -> Option<Arc<[u8]>> {
        self.read_vec_uncached(name).map(Arc::from)
    }

    fn read_vec_uncached(&self, name: &str) -> Option<Vec<u8>> {
        let (index, _) = *self.entries.get(name)?;
        let mut zip = self.zip.lock().ok()?;
        let mut entry = zip.by_index(index).ok()?;
        let mut data = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut data).ok()?;
        Some(data)
    }

    fn read_text(&self, name: &str) -> Option<String> {
        String::from_utf8(self.read_vec_uncached(name)?).ok()
    }
}

#[derive(Debug, Clone)]
pub struct AssetSource {
    root: PathBuf,
    canonical_root: Option<PathBuf>,
    files: HashMap<String, Arc<[u8]>>,
    archive: Option<Arc<Archive>>,
    directory: bool,
}

impl AssetSource {
    pub fn from_pak(path: &Path) -> Result<Self, PakError> {
        let bytes = std::fs::read(path).map_err(|source| PakError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_pak_bytes(bytes)
    }

    pub fn from_pak_bytes(bytes: Vec<u8>) -> Result<Self, PakError> {
        let bytes: Arc<[u8]> = Arc::from(bytes);
        let mut archive = zip::ZipArchive::new(Cursor::new(Arc::clone(&bytes)))?;
        let mut entries = HashMap::new();

        for index in 0..archive.len() {
            let entry = archive.by_index_raw(index)?;
            if entry.is_dir() {
                continue;
            }
            let raw_name = entry.name()?.into_owned();
            let Some(name) = normalize(&raw_name) else {
                return Err(PakError::UnsafeEntry(raw_name));
            };
            if name.is_empty() {
                continue;
            }
            entries.insert(name, (index, entry.size()));
        }

        Ok(Self {
            root: PathBuf::new(),
            canonical_root: None,
            files: HashMap::new(),
            archive: Some(Arc::new(Archive {
                entries,
                zip: Mutex::new(archive),
                cache: Mutex::new(EntryCache::new(CACHE_LIMIT)),
            })),
            directory: false,
        })
    }

    pub fn from_dir(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            canonical_root: root.canonicalize().ok(),
            files: HashMap::new(),
            archive: None,
            directory: true,
        }
    }

    pub fn is_directory(&self) -> bool {
        self.directory
    }

    pub fn exists(&self, path: &str) -> bool {
        let Some(key) = normalize(path) else {
            return false;
        };
        if self.directory {
            resolve_within(&self.root, self.canonical_root.as_deref(), &key)
                .is_some_and(|path| path.is_file())
        } else if let Some(archive) = &self.archive {
            archive.entries.contains_key(&key)
        } else {
            self.files.contains_key(&key)
        }
    }

    pub fn size(&self, path: &str) -> Option<usize> {
        let key = normalize(path)?;
        if self.directory {
            let full = resolve_within(&self.root, self.canonical_root.as_deref(), &key)?;
            std::fs::metadata(full)
                .ok()
                .map(|metadata| metadata.len() as usize)
        } else if let Some(archive) = &self.archive {
            archive.entries.get(&key).map(|(_, size)| *size as usize)
        } else {
            self.files.get(&key).map(|bytes| bytes.len())
        }
    }

    pub fn read(&self, path: &str) -> Option<Arc<[u8]>> {
        let key = normalize(path)?;
        if self.directory {
            let full = resolve_within(&self.root, self.canonical_root.as_deref(), &key)?;
            std::fs::read(full).ok().map(Arc::from)
        } else if let Some(archive) = &self.archive {
            archive.read(&key)
        } else {
            self.files.get(&key).cloned()
        }
    }

    pub fn read_uncached(&self, path: &str) -> Option<Arc<[u8]>> {
        let key = normalize(path)?;
        if self.directory {
            let full = resolve_within(&self.root, self.canonical_root.as_deref(), &key)?;
            std::fs::read(full).ok().map(Arc::from)
        } else if let Some(archive) = &self.archive {
            archive.read_uncached(&key)
        } else {
            self.files.get(&key).cloned()
        }
    }

    pub fn read_text(&self, path: &str) -> Option<String> {
        let key = normalize(path)?;
        if self.directory {
            let full = resolve_within(&self.root, self.canonical_root.as_deref(), &key)?;
            let bytes = std::fs::read(full).ok()?;
            String::from_utf8(bytes).ok()
        } else if let Some(archive) = &self.archive {
            archive.read_text(&key)
        } else {
            let bytes = self.files.get(&key)?;
            String::from_utf8(bytes.to_vec()).ok()
        }
    }

    pub fn file_count(&self) -> usize {
        self.archive
            .as_ref()
            .map_or(self.files.len(), |archive| archive.entries.len())
    }
}

pub fn normalize(path: &str) -> Option<String> {
    let mut clean = path.replace('\\', "/");
    if let Some(query) = clean.find(['?', '#']) {
        clean.truncate(query);
    }
    let clean = clean.trim_start_matches('/');
    let mut parts: Vec<&str> = Vec::new();

    for part in clean.split('/') {
        match part {
            "" | "." => continue,
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }

    Some(parts.join("/"))
}

fn resolve_within(root: &Path, canonical_root: Option<&Path>, relative: &str) -> Option<PathBuf> {
    let canonical_root = canonical_root?;
    let candidate = root.join(relative);
    let canonical = candidate.canonicalize().ok()?;
    canonical.starts_with(canonical_root).then_some(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn normalizes_paths() {
        assert_eq!(normalize("/sprites/bean.png").unwrap(), "sprites/bean.png");
        assert_eq!(normalize("./a/./b/../c.png").unwrap(), "a/c.png");
        assert_eq!(normalize("a//b\\c.png").unwrap(), "a/b/c.png");
        assert_eq!(
            normalize("sprites/bean.png?v=2").unwrap(),
            "sprites/bean.png"
        );
        assert_eq!(normalize("a/b.png#frag").unwrap(), "a/b.png");
        assert_eq!(normalize("../escape.png"), None);
        assert_eq!(normalize("a/../../escape.png"), None);
    }

    #[test]
    fn reads_from_directory() {
        let dir = std::env::temp_dir().join(format!("kap-pak-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sprites")).unwrap();
        std::fs::write(dir.join("sprites/bean.png"), b"png").unwrap();

        let assets = AssetSource::from_dir(&dir);
        assert!(assets.is_directory());
        assert_eq!(&*assets.read("sprites/bean.png").unwrap(), b"png");
        assert_eq!(assets.read_text("sprites/bean.png").unwrap(), "png");
        assert!(!assets.exists("nope.png"));
        assert!(assets.read("../escape.png").is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reads_from_pak() {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("game/main.js", options).unwrap();
        writer.write_all(b"console.log(1)").unwrap();
        writer.start_file("assets/bean.png", options).unwrap();
        writer.write_all(b"png").unwrap();
        let bytes = writer.finish().unwrap().into_inner();

        let dir = std::env::temp_dir().join(format!("kap-pak-zip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pak_path = dir.join("game.pak");
        std::fs::write(&pak_path, bytes).unwrap();

        let assets = AssetSource::from_pak(&pak_path).unwrap();
        assert!(!assets.is_directory());
        assert_eq!(assets.file_count(), 2);
        assert_eq!(&*assets.read("assets/bean.png").unwrap(), b"png");
        assert_eq!(&*assets.read_uncached("assets/bean.png").unwrap(), b"png");
        assert!(assets.exists("game/main.js"));
        assert!(assets.read("missing.js").is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cache_evicts_oldest_entries() {
        let mut cache = EntryCache::new(10);
        cache.insert("first", Arc::from(&b"123456"[..]));
        cache.insert("second", Arc::from(&b"789012"[..]));
        assert!(cache.get("first").is_none());
        assert_eq!(&*cache.get("second").unwrap(), b"789012");
        cache.insert("too-large", Arc::from(&b"012345678901"[..]));
        assert!(cache.get("too-large").is_none());
        assert_eq!(cache.bytes, 6);
    }
}
