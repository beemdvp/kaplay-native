use std::path::{Path, PathBuf};

use crate::config::HostConfig;

pub fn install(config: &HostConfig, comment: Option<&str>) {
    if config.headless || config.frames.is_some() || config.app_id.is_some() {
        return;
    }
    if std::env::var_os("KAPLAY_NATIVE_NO_DESKTOP").is_some() {
        return;
    }
    let Some(icon) = config.icon.as_deref() else {
        return;
    };
    let Some(data_home) = data_home() else {
        return;
    };
    let Some(app_id) = desktop_id(&config.app_name) else {
        return;
    };
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    if let Err(error) = write_files(
        &data_home,
        &app_id,
        &config.game_name,
        comment.unwrap_or(&config.game_name),
        &exe,
        icon,
    ) {
        log::warn!("could not install the desktop entry: {error}");
    }
}

fn data_home() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            return Some(path);
        }
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share"))
}

fn desktop_id(app_name: &str) -> Option<String> {
    let id = app_name.trim();
    if id.is_empty() || id.chars().any(|c| matches!(c, '/' | '\\' | '\0')) {
        return None;
    }
    Some(id.to_string())
}

fn write_files(
    data_home: &Path,
    app_id: &str,
    name: &str,
    comment: &str,
    exe: &Path,
    icon: &[u8],
) -> std::io::Result<()> {
    let icon_path = data_home
        .join("icons")
        .join("hicolor")
        .join("128x128")
        .join("apps")
        .join(format!("{app_id}.png"));
    let entry_path = data_home
        .join("applications")
        .join(format!("{app_id}.desktop"));
    let icon_changed = write_if_changed(&icon_path, icon)?;
    let entry_changed = write_if_changed(
        &entry_path,
        entry_contents(name, comment, app_id, exe).as_bytes(),
    )?;
    if icon_changed || entry_changed {
        log::info!("installed desktop entry {}", entry_path.display());
    }
    Ok(())
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> std::io::Result<bool> {
    if std::fs::read(path).is_ok_and(|existing| existing == bytes) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    Ok(true)
}

fn entry_contents(name: &str, comment: &str, app_id: &str, exe: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={}\n\
         Comment={}\n\
         Exec={}\n\
         Icon={}\n\
         Terminal=false\n\
         Categories=Game;\n\
         StartupWMClass={}\n\
         StartupNotify=true\n\
         X-Kaplay-Native=1\n",
        field(name),
        field(comment),
        exec(exe),
        app_id,
        app_id
    )
}

fn field(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn exec(path: &Path) -> String {
    let path = path.to_string_lossy();
    let escaped = path
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`")
        .replace('%', "%%")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kaplay-native-desktop-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writes_the_entry_and_icon() {
        let home = temp_home("entry");
        let exe = Path::new("/opt/Frax Rupture/game");
        write_files(
            &home,
            "gg.frax.rupture",
            "Frax Rupture",
            "Free Pixelart Isometric MMORPG",
            exe,
            b"png bytes",
        )
        .unwrap();

        let entry =
            std::fs::read_to_string(home.join("applications/gg.frax.rupture.desktop")).unwrap();
        assert!(entry.contains("Name=Frax Rupture\n"));
        assert!(entry.contains("Comment=Free Pixelart Isometric MMORPG\n"));
        assert!(entry.contains("Exec=\"/opt/Frax Rupture/game\"\n"));
        assert!(entry.contains("Icon=gg.frax.rupture\n"));
        assert!(entry.contains("Categories=Game;\n"));
        assert!(entry.contains("StartupWMClass=gg.frax.rupture\n"));
        assert!(entry.contains("X-Kaplay-Native=1\n"));
        assert_eq!(
            std::fs::read(home.join("icons/hicolor/128x128/apps/gg.frax.rupture.png")).unwrap(),
            b"png bytes"
        );
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn rewrites_the_entry_when_the_binary_moves() {
        let home = temp_home("move");
        write_files(
            &home,
            "game",
            "Game",
            "Game",
            Path::new("/tmp/one/game"),
            b"png",
        )
        .unwrap();
        write_files(
            &home,
            "game",
            "Game",
            "Game",
            Path::new("/tmp/two/game"),
            b"png",
        )
        .unwrap();
        let entry = std::fs::read_to_string(home.join("applications/game.desktop")).unwrap();
        assert!(entry.contains("Exec=\"/tmp/two/game\"\n"));
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn leaves_an_unchanged_entry_alone() {
        let home = temp_home("unchanged");
        let exe = Path::new("/tmp/game");
        write_files(&home, "game", "Game", "Game", exe, b"png").unwrap();
        let path = home.join("applications/game.desktop");
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        write_files(&home, "game", "Game", "Game", exe, b"png").unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified
        );
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn escapes_desktop_values_and_exec_paths() {
        assert_eq!(field("a\\b\nc"), "a\\\\b\\nc");
        assert_eq!(
            exec(Path::new("/tmp/a \"b\"/c$d")),
            "\"/tmp/a \\\"b\\\"/c\\$d\""
        );
    }

    #[test]
    fn rejects_ids_that_cannot_be_desktop_files() {
        assert_eq!(
            desktop_id("gg.frax.rupture").as_deref(),
            Some("gg.frax.rupture")
        );
        assert_eq!(desktop_id(""), None);
        assert_eq!(desktop_id("a/b"), None);
    }
}
