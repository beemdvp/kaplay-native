use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use kaplay_native_audio::Audio;
use kaplay_native_canvas2d::Canvas2d;
use kaplay_native_gfx::pixels::UploadOptions;
use kaplay_native_gfx::GlState;
use kaplay_native_net::{HttpResponse, Network};
use kaplay_native_pak::AssetSource;
use kaplay_native_steam::SteamApi;
use kaplay_native_video::Video;
use winit::window::Window;

pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub struct PendingMessage {
    pub socket: u32,
    pub data: Vec<u8>,
}

pub struct VideoInfo {
    pub id: u32,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
}

pub struct HostState {
    pub gl: Rc<RefCell<GlState>>,
    pub canvas: Rc<RefCell<Canvas2d>>,
    pub images: RefCell<HashMap<u32, ImageData>>,
    pub videos: RefCell<HashMap<u32, Video>>,
    pub next_image: Cell<u32>,
    pub next_video: Cell<u32>,
    pub assets: Rc<AssetSource>,
    pub window: Arc<Window>,
    pub quit: Cell<bool>,
    pub fullscreen: Cell<bool>,
    pub cursor_locked: Cell<bool>,
    pub cursor: RefCell<String>,
    pub start: Instant,
    pub save_path: PathBuf,
    pub platform: &'static str,
    pub host_version: &'static str,
    pub allow_network: bool,
    pub start_url: String,
    pub network: Option<Rc<Network>>,
    pub audio: Option<Rc<Audio>>,
    pub pending_response: RefCell<Option<HttpResponse>>,
    pub pending_message: RefCell<Option<PendingMessage>>,
    pub auth: RefCell<Option<crate::auth::AuthServer>>,
    pub game_name: String,
    pub steam: Rc<RefCell<Option<SteamApi>>>,
    pub updates: Arc<crate::updates::UpdateState>,
}

impl HostState {
    pub fn now_ms(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    pub fn store_image(&self, width: u32, height: u32, pixels: Vec<u8>) -> u32 {
        let id = self.next_image.get();
        self.next_image.set(id + 1);
        self.images.borrow_mut().insert(
            id,
            ImageData {
                width,
                height,
                pixels,
            },
        );
        id
    }

    pub fn open_video(&self, bytes: Arc<[u8]>) -> Option<VideoInfo> {
        let video = Video::open(bytes)
            .map_err(|error| log::warn!("video: {error}"))
            .ok()?;
        let id = self.next_video.get();
        self.next_video.set(id + 1);
        let info = VideoInfo {
            id,
            width: video.width(),
            height: video.height(),
            duration: video.duration(),
        };
        self.videos.borrow_mut().insert(id, video);
        Some(info)
    }

    pub fn open_video_asset(&self, path: &str) -> Option<VideoInfo> {
        let Some(bytes) = self.assets.read(path) else {
            log::warn!("video asset not found: {path}");
            return None;
        };
        self.open_video(bytes)
    }

    pub fn decode_audio_asset(&self, request: u32, path: &str) {
        let Some(audio) = self.audio.as_ref() else {
            return;
        };
        let bytes = self.assets.read(path).unwrap_or_else(|| Arc::from(&[][..]));
        audio.decode_async(request, bytes);
    }

    pub fn close_video(&self, id: u32) {
        self.videos.borrow_mut().remove(&id);
    }

    pub fn video_frame_ready(&self, id: u32, time: f64) -> bool {
        self.videos
            .borrow_mut()
            .get_mut(&id)
            .is_some_and(|video| video.frame_ready(time))
    }

    pub fn upload_video_frame(
        &self,
        id: u32,
        time: f64,
        target: u32,
        internal_format: i32,
        format: u32,
        ty: u32,
    ) {
        let mut videos = self.videos.borrow_mut();
        let Some(video) = videos.get_mut(&id) else {
            return;
        };
        if let Some(error) = video.take_error() {
            log::warn!("video frame: {error}");
        }
        let frame = video.frame_at(time);
        let options = UploadOptions {
            flip_y: self.gl.borrow().upload_options().flip_y,
            premultiply: false,
        };
        self.gl.borrow_mut().tex_image_2d(
            target,
            0,
            internal_format,
            frame.width as i32,
            frame.height as i32,
            0,
            format,
            ty,
            Some(&frame.pixels),
            options,
        );
    }

    pub fn decode_image(&self, bytes: &[u8]) -> Option<u32> {
        let image = image::load_from_memory(bytes).ok()?;
        let rgba = image.to_rgba8();
        Some(self.store_image(rgba.width(), rgba.height(), rgba.into_raw()))
    }

    pub fn load_storage(&self) -> String {
        std::fs::read_to_string(&self.save_path).unwrap_or_default()
    }

    pub fn save_storage(&self, data: &str) {
        if let Some(parent) = self.save_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let temporary = self.save_path.with_extension("tmp");
        if std::fs::write(&temporary, data).is_ok() {
            let _ = std::fs::rename(&temporary, &self.save_path);
        }
    }

    pub fn save_download(&self, name: &str, bytes: &[u8]) -> Option<PathBuf> {
        let mut directories = Vec::new();
        if let Some(directory) = dirs::download_dir() {
            directories.push(directory);
        }
        if let Some(home) = dirs::home_dir() {
            directories.push(home.join("Downloads"));
        }
        if let Some(parent) = self.save_path.parent() {
            directories.push(parent.to_path_buf());
        }
        directories
            .into_iter()
            .find_map(|directory| write_download(&directory, name, bytes))
    }

    pub fn set_cursor(&self, cursor: &str) {
        log::debug!("cursor: {cursor}");
        *self.cursor.borrow_mut() = cursor.to_string();
        if cursor == "none" {
            self.window.set_cursor_visible(false);
            return;
        }
        self.window.set_cursor_visible(true);
        self.window.set_cursor(cursor_icon(cursor));
    }

    pub fn set_cursor_locked(&self, locked: bool) {
        self.cursor_locked.set(locked);
        if locked {
            let _ = self
                .window
                .set_cursor_grab(winit::window::CursorGrabMode::Locked)
                .or_else(|_| {
                    self.window
                        .set_cursor_grab(winit::window::CursorGrabMode::Confined)
                });
            self.window.set_cursor_visible(false);
        } else {
            let _ = self
                .window
                .set_cursor_grab(winit::window::CursorGrabMode::None);
            self.window.set_cursor_visible(true);
        }
    }
}

fn write_download(directory: &Path, name: &str, bytes: &[u8]) -> Option<PathBuf> {
    std::fs::create_dir_all(directory).ok()?;
    let path = unique_download_path(directory, download_name(name));
    std::fs::write(&path, bytes).ok()?;
    Some(path)
}

fn download_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    let filtered: String = base
        .chars()
        .filter(|character| {
            !character.is_control() && !matches!(character, ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        .collect();
    if filtered.is_empty() || filtered == "." || filtered == ".." {
        "download".to_string()
    } else {
        filtered
    }
}

fn unique_download_path(directory: &Path, name: String) -> PathBuf {
    let candidate = directory.join(&name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(&name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("download");
    let extension = path.extension().and_then(|value| value.to_str());
    for index in 1.. {
        let file = match extension {
            Some(extension) => format!("{stem} ({index}).{extension}"),
            None => format!("{stem} ({index})"),
        };
        let candidate = directory.join(file);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

fn cursor_icon(cursor: &str) -> winit::window::CursorIcon {
    use winit::window::CursorIcon;
    match cursor {
        "crosshair" => CursorIcon::Crosshair,
        "pointer" => CursorIcon::Pointer,
        "text" => CursorIcon::Text,
        "move" => CursorIcon::Move,
        "not-allowed" => CursorIcon::NotAllowed,
        "grab" => CursorIcon::Grab,
        "grabbing" => CursorIcon::Grabbing,
        "ew-resize" => CursorIcon::EwResize,
        "ns-resize" => CursorIcon::NsResize,
        "nesw-resize" => CursorIcon::NeswResize,
        "nwse-resize" => CursorIcon::NwseResize,
        "wait" => CursorIcon::Wait,
        "none" => CursorIcon::Default,
        _ => CursorIcon::Default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WEBP: &str = "UklGRjwAAABXRUJQVlA4IDAAAADQAQCdASoBAAEAAgA0JaACdLoB+AADsAD+8Oj3/yC5YXXI1/8gP+QH/ID/+PIAAAA=";
    const GIF: &str = "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
    const BMP: &str =
        "Qk06AAAAAAAAADYAAAAoAAAAAQAAAAEAAAABABgAAAAAAAQAAAAAAAAAAAAAAAAAAAAAAAAAAAD+AA==";

    fn image_bytes(value: &str) -> Vec<u8> {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(value)
            .unwrap()
    }

    #[test]
    fn downloads_keep_names_and_avoid_overwrites() {
        let directory =
            std::env::temp_dir().join(format!("kaplay-native-download-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);

        let first = write_download(&directory, "../save.txt", b"one").unwrap();
        assert_eq!(first.file_name().unwrap(), "save.txt");
        let second = write_download(&directory, "save.txt", b"two").unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(first).unwrap(), b"one");
        assert_eq!(std::fs::read(second).unwrap(), b"two");

        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn image_decoder_accepts_webp_gif_and_bmp() {
        for bytes in [image_bytes(WEBP), image_bytes(GIF), image_bytes(BMP)] {
            let image = image::load_from_memory(&bytes).unwrap();
            assert_eq!((image.width(), image.height()), (1, 1));
        }
    }
}
