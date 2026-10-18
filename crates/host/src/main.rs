mod app;
mod auth;
mod bridge;
mod config;
mod embed;
mod js;
mod keys;
mod screenshots;
mod state;
mod updates;

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use kaplay_native_pak::AssetSource;

use crate::app::HostApp;
use crate::config::HostConfig;

#[derive(Debug, Parser)]
#[command(name = "kaplay-native-host", version, about = "KAPLAY native runtime")]
struct Args {
    #[arg(long, help = "Game script evaluated after the runtime shim")]
    script: Option<PathBuf>,
    #[arg(long, help = "Preload script evaluated before the game script", action = clap::ArgAction::Append)]
    preload: Vec<PathBuf>,
    #[arg(long, help = "Read assets from a .pak zip archive")]
    pak: Option<PathBuf>,
    #[arg(long, help = "Read assets from a directory")]
    dir: Option<PathBuf>,
    #[arg(long, help = "Steam application id")]
    appid: Option<u32>,
    #[arg(long, help = "Initial window size (default: 960x540)")]
    window: Option<String>,
    #[arg(long)]
    fullscreen: bool,
    #[arg(long, help = "Run without showing the window")]
    headless: bool,
    #[arg(long, help = "Exit after this many frames")]
    frames: Option<u32>,
    #[arg(long, help = "Write a PNG of the last frame before exiting")]
    screenshot: Option<PathBuf>,
    #[arg(long, help = "Write the last frame as raw top-down RGBA bytes")]
    screenshot_raw: Option<PathBuf>,
    #[arg(
        long,
        value_name = "PATH",
        help = "Write a JSON benchmark summary before exiting"
    )]
    bench_json: Option<PathBuf>,
    #[arg(
        long,
        value_name = "PATH",
        help = "Write the per-frame timings (last 10000 frames) as JSON before exiting"
    )]
    frame_trace: Option<PathBuf>,
    #[arg(long, help = "Directory for the localStorage save file")]
    save_dir: Option<PathBuf>,
    #[arg(long, help = "Allow http(s) requests (disabled by default)")]
    allow_network: bool,
    #[arg(
        long,
        help = "Page URL games see as window.location (used to resolve relative requests)"
    )]
    url: Option<String>,
    #[arg(long, help = "Disable Steam integration")]
    no_steam: bool,
    #[arg(long, help = "Delete installed content updates before starting")]
    reset_content: bool,
    #[arg(long, help = "Skip the content update check")]
    no_update: bool,
    #[arg(long, help = "Run without audio output")]
    no_audio: bool,
    #[arg(long, help = "Disable vertical sync (uncapped frame rate)")]
    no_vsync: bool,
    #[arg(long, help = "Log the frame rate while the game runs")]
    stats: bool,
    #[arg(long, help = "Prefer the X11 backend on Linux")]
    x11: bool,
    #[arg(long, help = "Window title (default: KAPLAY or the packaged default)")]
    title: Option<String>,
    #[arg(long, value_name = "NAME", help = "Javascript engine (quickjs or v8)")]
    js_engine: Option<String>,
    #[arg(long, help = "Print the compiled-in javascript engines and exit")]
    engines: bool,
}

fn parse_size(value: &str) -> (u32, u32) {
    let mut parts = value.split('x');
    let width = parts
        .next()
        .and_then(|part| part.trim().parse().ok())
        .unwrap_or(960);
    let height = parts
        .next()
        .and_then(|part| part.trim().parse().ok())
        .unwrap_or(540);
    (width, height)
}

fn packaged_source(embedded: Option<embed::Embedded>, args: &Args) -> anyhow::Result<AssetSource> {
    match embedded {
        Some(entry) => Ok(AssetSource::from_pak_bytes(entry.pak)?),
        None => {
            let root = args
                .script
                .as_ref()
                .and_then(|path| path.parent())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            Ok(AssetSource::from_dir(&root))
        }
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();

    if args.engines {
        for engine in js::available_engines() {
            println!("{}", engine.name());
        }
        return Ok(());
    }

    let mut embedded = embed::read_current();
    let pack = embedded
        .as_ref()
        .map(|entry| entry.config.clone())
        .unwrap_or_default();

    let requested = args.js_engine.clone().or_else(|| pack.js_engine.clone());
    let js_engine = match requested {
        Some(name) => match name.parse::<js::JsEngine>() {
            Ok(engine) if js::available_engines().contains(&engine) => engine,
            Ok(engine) => anyhow::bail!(
                "javascript engine '{engine}' is not available in this build (available: {})",
                js::engine_list()
            ),
            Err(error) => anyhow::bail!("{error}"),
        },
        None => js::default_engine(),
    };

    let save_dir = args.save_dir.clone().unwrap_or_else(|| {
        let root = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("kaplay-native");
        match pack.game_id.as_deref().and_then(updates::safe_component) {
            Some(game_id) => root.join(game_id),
            None => root,
        }
    });

    if args.reset_content {
        updates::reset(&save_dir);
    }

    let mut installed =
        if args.pak.is_none() && args.dir.is_none() && pack.content_version.is_some() {
            updates::preferred(
                updates::installed(&save_dir),
                pack.content_version.as_deref(),
            )
        } else {
            None
        };

    let source = match (&args.pak, &args.dir) {
        (Some(pak), _) => AssetSource::from_pak(pak)?,
        (None, Some(dir)) => AssetSource::from_dir(dir),
        (None, None) => match installed.take() {
            Some(content) => match AssetSource::from_pak(&updates::pak_path(&save_dir, &content)) {
                Ok(source) => {
                    log::info!("using installed content {}", content.version);
                    installed = Some(content);
                    source
                }
                Err(error) => {
                    log::warn!(
                        "installed content {} could not be loaded: {error}",
                        content.version
                    );
                    updates::reset(&save_dir);
                    packaged_source(embedded.take(), &args)?
                }
            },
            None => packaged_source(embedded.take(), &args)?,
        },
    };

    let content_version = installed
        .as_ref()
        .map(|content| content.version.clone())
        .or_else(|| pack.content_version.clone());
    let update_state = Arc::new(updates::UpdateState::new(content_version.clone()));

    if !args.no_update
        && !args.headless
        && args.pak.is_none()
        && args.dir.is_none()
        && args.appid.is_none()
        && std::env::var_os("KAPLAY_NATIVE_NO_UPDATE").is_none()
        && std::env::var_os("SteamAppId").is_none()
    {
        if let (Some(settings), Some(game_id)) = (pack.update.clone(), pack.game_id.clone()) {
            let current_version = content_version
                .clone()
                .unwrap_or_else(|| "0.0.0".to_string());
            updates::spawn_check(
                settings,
                save_dir.clone(),
                game_id,
                current_version,
                Arc::clone(&update_state),
            );
        }
    }

    let window = args
        .window
        .as_deref()
        .or(pack.window.as_deref())
        .map(parse_size)
        .unwrap_or((960, 540));

    let config = HostConfig {
        script: args.script,
        preload: args.preload,
        app_id: args.appid,
        window,
        fullscreen: args.fullscreen,
        allow_network: args.allow_network || pack.network.unwrap_or(false),
        no_audio: args.no_audio,
        no_vsync: args.no_vsync,
        stats: args.stats,
        x11: args.x11,
        steam: !args.no_steam,
        headless: args.headless,
        frames: args.frames,
        screenshot: args.screenshot,
        screenshot_raw: args.screenshot_raw,
        bench_json: args.bench_json,
        frame_trace: args.frame_trace,
        save_dir,
        game_name: args
            .title
            .or(pack.title)
            .unwrap_or_else(|| "KAPLAY".to_string()),
        url: args.url.or(pack.url),
        js_engine,
        updates: update_state,
    };

    HostApp::new(config, std::rc::Rc::new(source))?.run()
}
