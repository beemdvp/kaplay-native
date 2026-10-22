use std::path::{Component, Path, PathBuf};

use kaplay_native_pak::{normalize, AssetSource};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleId {
    Asset(String),
    File(PathBuf),
}

impl ModuleId {
    pub fn from_name(name: &str) -> Self {
        if Path::new(name).is_absolute() {
            Self::File(PathBuf::from(name))
        } else {
            Self::Asset(normalize(name).unwrap_or_else(|| name.replace('\\', "/")))
        }
    }

    pub fn name(&self) -> String {
        match self {
            Self::Asset(path) => path.clone(),
            Self::File(path) => path.to_string_lossy().into_owned(),
        }
    }
}

pub const MODULE_ORIGIN: &str = "kaplay-native://game/";

pub fn module_url(id: &ModuleId) -> String {
    match id {
        ModuleId::Asset(path) => format!("{MODULE_ORIGIN}{path}"),
        ModuleId::File(path) => format!("file://{}", path.display()),
    }
}

pub fn resolve_specifier(base: &ModuleId, specifier: &str) -> Result<ModuleId, String> {
    let specifier = specifier.trim();
    let clean = strip_query(specifier);
    if clean.is_empty() {
        return Err(format!(
            "cannot resolve an empty module specifier imported by '{}'",
            base.name()
        ));
    }
    if has_scheme(clean) {
        return Err(format!(
            "cannot load module '{clean}' imported by '{}': network and data URLs are not supported",
            base.name()
        ));
    }
    if let Some(path) = clean.strip_prefix('/') {
        return normalize(path)
            .map(ModuleId::Asset)
            .ok_or_else(|| escapes_root(clean, base));
    }
    if clean.starts_with("./") || clean.starts_with("../") {
        return match base {
            ModuleId::Asset(referrer) => {
                let directory = referrer
                    .rsplit_once('/')
                    .map_or("", |(directory, _)| directory);
                let joined = match directory {
                    "" => clean.to_string(),
                    directory => format!("{directory}/{clean}"),
                };
                normalize(&joined)
                    .map(ModuleId::Asset)
                    .ok_or_else(|| escapes_root(&joined, base))
            }
            ModuleId::File(referrer) => {
                let directory = referrer.parent().unwrap_or_else(|| Path::new(""));
                Ok(ModuleId::File(lexical(directory.join(clean))))
            }
        };
    }
    Err(format!(
        "cannot resolve module specifier '{clean}' imported by '{}': relative ('./x.js'), parent ('../x.js'), and root ('/x.js') specifiers load from the game files, so bare specifiers must be bundled",
        base.name()
    ))
}

pub fn read_module(assets: &AssetSource, id: &ModuleId) -> Result<Vec<u8>, String> {
    match id {
        ModuleId::Asset(path) => assets
            .read_uncached(path)
            .map(|bytes| bytes.to_vec())
            .ok_or_else(|| format!("module '{path}' was not found in the game files")),
        ModuleId::File(path) => std::fs::read(path)
            .map_err(|error| format!("failed to read module '{}': {error}", id.name())),
    }
}

fn strip_query(specifier: &str) -> &str {
    match specifier.find(['?', '#']) {
        Some(index) => &specifier[..index],
        None => specifier,
    }
}

fn has_scheme(specifier: &str) -> bool {
    if specifier.starts_with("//") {
        return true;
    }
    let Some((scheme, _)) = specifier.split_once(':') else {
        return false;
    };
    !scheme.is_empty()
        && scheme.starts_with(|first: char| first.is_ascii_alphabetic())
        && scheme.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
}

fn escapes_root(path: &str, base: &ModuleId) -> String {
    format!(
        "cannot resolve module specifier '{path}' imported by '{}': it escapes the game files",
        base.name()
    )
}

fn lexical(path: PathBuf) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            component => result.push(component.as_os_str()),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{module_url, read_module, resolve_specifier, ModuleId};
    use kaplay_native_pak::AssetSource;

    fn asset(path: &str) -> ModuleId {
        ModuleId::Asset(path.to_string())
    }

    fn file_base(path: &str) -> ModuleId {
        ModuleId::File(std::env::temp_dir().join("kap-module-test").join(path))
    }

    #[test]
    fn resolves_relative_asset_specifiers() {
        assert_eq!(
            resolve_specifier(&asset("assets/index-abc.js"), "./kaplay-def.js").unwrap(),
            asset("assets/kaplay-def.js")
        );
        assert_eq!(
            resolve_specifier(&asset("assets/index-abc.js"), "./nested/chunk.js").unwrap(),
            asset("assets/nested/chunk.js")
        );
        assert_eq!(
            resolve_specifier(&asset("assets/index-abc.js"), "../shared.js").unwrap(),
            asset("shared.js")
        );
    }

    #[test]
    fn resolves_root_asset_specifiers() {
        assert_eq!(
            resolve_specifier(&asset("assets/index-abc.js"), "/assets/kaplay-def.js").unwrap(),
            asset("assets/kaplay-def.js")
        );
    }

    #[test]
    fn strips_queries_and_fragments() {
        assert_eq!(
            resolve_specifier(&asset("index.js"), "./chunk.js?v=2").unwrap(),
            asset("chunk.js")
        );
        assert_eq!(
            resolve_specifier(&asset("index.js"), "./chunk.js#frag").unwrap(),
            asset("chunk.js")
        );
    }

    #[test]
    fn resolves_relative_file_specifiers() {
        let base = file_base("demo/main.js");
        assert_eq!(
            resolve_specifier(&base, "./lib/chunk.js").unwrap(),
            file_base("demo/lib/chunk.js")
        );
        assert_eq!(
            resolve_specifier(&base, "../shared.js").unwrap(),
            file_base("shared.js")
        );
    }

    #[test]
    fn root_specifiers_always_load_from_the_assets() {
        let base = file_base("demo/main.js");
        assert_eq!(
            resolve_specifier(&base, "/assets/chunk.js").unwrap(),
            asset("assets/chunk.js")
        );
    }

    #[test]
    fn rejects_bare_and_remote_specifiers() {
        let error = resolve_specifier(&asset("assets/index.js"), "kaplay").unwrap_err();
        assert!(error.contains("must be bundled"), "{error}");
        let error =
            resolve_specifier(&asset("assets/index.js"), "https://cdn.test/x.js").unwrap_err();
        assert!(error.contains("not supported"), "{error}");
        let error =
            resolve_specifier(&asset("assets/index.js"), "data:text/javascript,").unwrap_err();
        assert!(error.contains("not supported"), "{error}");
    }

    #[test]
    fn rejects_specifiers_that_escape_the_game_files() {
        let error = resolve_specifier(&asset("assets/index.js"), "../../escape.js").unwrap_err();
        assert!(error.contains("escapes"), "{error}");
        let error = resolve_specifier(&asset("assets/index.js"), "/../escape.js").unwrap_err();
        assert!(
            error.contains("escapes") || error.contains("cannot resolve"),
            "{error}"
        );
    }

    #[test]
    fn derives_module_ids_from_names() {
        assert_eq!(
            ModuleId::from_name("assets/index.js"),
            asset("assets/index.js")
        );
        assert_eq!(ModuleId::from_name("assets/../chunk.js"), asset("chunk.js"));
        let absolute = std::env::temp_dir()
            .join("kap-module-test")
            .join("demo/main.js");
        assert_eq!(
            ModuleId::from_name(&absolute.to_string_lossy()),
            ModuleId::File(absolute)
        );
    }

    #[test]
    fn derives_module_urls() {
        assert_eq!(
            module_url(&asset("assets/chunk.js")),
            "kaplay-native://game/assets/chunk.js"
        );
        let file = std::env::temp_dir().join("kap-module-test/demo/main.js");
        assert_eq!(
            module_url(&ModuleId::File(file.clone())),
            format!("file://{}", file.display())
        );
    }

    #[test]
    fn reads_asset_and_file_modules() {
        let dir = std::env::temp_dir().join(format!("kap-module-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("assets/chunk.js"), b"export const x = 1;").unwrap();
        let assets = AssetSource::from_dir(&dir);
        assert_eq!(
            read_module(&assets, &asset("assets/chunk.js")).unwrap(),
            b"export const x = 1;"
        );
        assert!(read_module(&assets, &asset("assets/missing.js"))
            .unwrap_err()
            .contains("not found"));
        let file = dir.join("assets/chunk.js");
        assert_eq!(
            read_module(&assets, &ModuleId::File(file)).unwrap(),
            b"export const x = 1;"
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
