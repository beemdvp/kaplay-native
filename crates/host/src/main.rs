mod app;
mod auth;
mod bridge;
mod config;
mod embed;
mod js;
mod keys;
mod screenshots;
mod state;

use std::path::PathBuf;

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

    let source = match (&args.pak, &args.dir) {
        (Some(pak), _) => AssetSource::from_pak(pak)?,
        (None, Some(dir)) => AssetSource::from_dir(dir),
        (None, None) => match embedded.take() {
            Some(entry) => AssetSource::from_pak_bytes(entry.pak)?,
            None => {
                let root = args
                    .script
                    .as_ref()
                    .and_then(|path| path.parent())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("."));
                AssetSource::from_dir(&root)
            }
        },
    };

    let save_dir = args.save_dir.unwrap_or_else(|| {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("kaplay-native")
    });

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
    };

    HostApp::new(config, std::rc::Rc::new(source))?.run()
}
