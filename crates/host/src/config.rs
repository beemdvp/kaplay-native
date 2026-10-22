use std::path::PathBuf;
use std::sync::Arc;

use crate::js::JsEngine;
use crate::updates::UpdateState;

#[derive(Debug, Clone)]
pub struct HostConfig {
    pub script: Option<PathBuf>,
    pub preload: Vec<PathBuf>,
    pub app_id: Option<u32>,
    pub window: (u32, u32),
    pub fullscreen: bool,
    pub allow_network: bool,
    pub no_audio: bool,
    pub no_vsync: bool,
    pub stats: bool,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub x11: bool,
    pub steam: bool,
    pub headless: bool,
    pub frames: Option<u32>,
    pub screenshot: Option<PathBuf>,
    pub screenshot_raw: Option<PathBuf>,
    pub bench_json: Option<PathBuf>,
    pub frame_trace: Option<PathBuf>,
    pub resource_report: Option<PathBuf>,
    pub save_dir: PathBuf,
    pub game_name: String,
    pub url: Option<String>,
    pub js_engine: JsEngine,
    pub updates: Arc<UpdateState>,
}
