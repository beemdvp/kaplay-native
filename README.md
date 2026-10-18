# KAPLAY Native

Run [KAPLAY](https://kaplayjs.com) games as native desktop applications on
Windows, Linux, and macOS, with a working Steam overlay and Steamworks
integration.

KAPLAY Native is not a browser shell. It is a small Rust host that owns the
window, the GL context, and the frame loop, implements the browser APIs KAPLAY
uses, and evaluates the unmodified KAPLAY bundle with a built-in JavaScript
engine. Nothing in KAPLAY is patched: the runtime installs the globals, then
loads the game exactly as a browser would.

```
game script ──► QuickJS or V8 ──► KAPLAY bundle (unchanged)
                   │
                   ▼
          JS shim (DOM, canvas, WebGL, fetch, storage)
                   │
                   ▼
        Rust host (winit + glutin + tiny-skia + Steam)
                   │
                   ▼
        window / OpenGL / disk / Steam overlay
```

## Status

Early. The runtime runs real KAPLAY games today: assets load, sprites and text
render through the GL pipeline, input, storage, and screenshots work. Steam
support is implemented behind runtime library loading but has not yet been
validated against a shipping Steam depot - see [docs/steam.md](docs/steam.md)
for the manual validation checklist.

- Runtime: window, GL 2.1 compatibility context, frame loop, aspect-fit present
- Bridge: WebGL 1, Canvas 2D, images, fonts, timers, events, `localStorage`,
  and an `index.html` document tree
- Input: keyboard, mouse, and standard-mapped gamepads with connect/disconnect
  events
- Network: `fetch` (HTTP/HTTPS) and `WebSocket` (ws/wss) on background threads
- Audio: Web Audio nodes over a native software mixer, plus `HTMLAudioElement`
  music, decoding WAV, MP3, FLAC, Ogg Vorbis, and Opus; `OscillatorNode` and
  `BiquadFilterNode` render short synthesized notes
- Video: `HTMLVideoElement` over a threaded, pure-Rust H.264 MP4 decoder,
  drawn with `gl.texImage2D` (the path KAPLAY's `video()` component uses)
- Assets: directory or `.pak` archives, plus remote images and fonts with
  `--allow-network`; PNG, JPEG, WebP, GIF, and BMP images decode natively
- Steam: dynamic `steam_api` loading, user, achievements, stats, cloud storage
- Engines: QuickJS by default, optional V8 backend for full modern JS and `Intl`
- CLI: `run`, `dev`, `build`, `pack`, `test`, `engines`, `doctor`

Not implemented yet: touch input, screen recording, gamepad haptics, and the
Steam Input action manifest. QuickJS builds have no `Intl`; use the V8 backend
for locale-aware APIs. See [docs/engines.md](docs/engines.md) for the engine
matrix.

## Quick start

Build the host and run the examples with the KAPLAY bundle from a checkout:

```sh
cargo build --release
node cli/src/index.mjs run examples/hello --preload path/to/kaplay.js
```

On Linux, building the host needs the `libudev` development headers:

```sh
sudo apt-get install libudev-dev
```

Both KAPLAY builds work as preloads: the classic `kaplay.js` defines its
globals directly, and the ES module `kaplay.mjs` gets its named exports copied
onto the global object.

Or run the host directly:

```sh
target/release/kaplay-native-host \
  --script examples/hello/main.js \
  --preload path/to/kaplay.js \
  --dir examples/hello \
  --window 960x540
```

With the npm CLI installed, no local checkout is needed:

```sh
npm install --global kaplay-native
kaplay-native pack ./my-game --js-engine v8
```

The headless smoke test uses raw WebGL, so it needs no preload:

```sh
target/release/kaplay-native-host --script examples/smoke/main.js --dir examples/smoke --frames 5 --screenshot smoke.png
```

## CLI

The CLI is a small Node script that locates the host binary, bundles the game
with esbuild when available, and packs assets into a `.pak`:

```sh
node cli/src/index.mjs run ./my-game
node cli/src/index.mjs run --web-dist ./my-game/dist
node cli/src/index.mjs dev ./my-game
node cli/src/index.mjs build ./my-game --out dist/my-game.pak
node cli/src/index.mjs test ./my-game --update
node cli/src/index.mjs pack ./my-game --out dist/my-game --js-engine v8
node cli/src/index.mjs engines
node cli/src/index.mjs doctor
```

The host binary is found with `--host`, `KAPLAY_NATIVE_HOST`, next to the CLI,
or on `PATH`.

## Distributing a game

`pack` turns a game into a single-file executable for the current platform: a
copy of the host with the game archive and a config record appended, so the
player needs nothing else. The config embeds the defaults a double-clicked game
cannot pass on the command line, including the engine, window title and size,
and network settings:

```sh
node cli/src/index.mjs pack ./my-game --out dist/my-game \
  --js-engine v8 --allow-network --url https://my-game.example.com
```

The same command is available from the published npm CLI, which downloads the
matching host binary on first use, and a GitHub Actions workflow can produce
artifacts for Linux, Windows, and macOS in one matrix. Builds, run-time
overrides, the packed format, and a copy-paste release workflow are documented
in [docs/distribution.md](docs/distribution.md). Signed content updates, which
let a packed game replace its assets and scripts on the next launch without
replacing the executable, are documented in [docs/updates.md](docs/updates.md).

## Running a web build

For games built with Vite or Rollup, run the production build directly. The
host reads `index.html`, runs the module script it references, and resolves
assets from the same directory, so the project's own Vite plugins (sprite
atlases, PWA registration, `virtual:` modules, aliases) apply exactly as they
do on the web:

```sh
node cli/src/index.mjs run --web-dist path/to/game/dist
```

or with the host binary directly:

```sh
kaplay-native-host --dir path/to/game/dist --window 1280x720
```

The entry script is evaluated as an ES module, like
`<script type="module">` in a browser. Top-level declarations stay
module-scoped, so they cannot collide with globals that KAPLAY or another
library exports onto `window`. Imports between chunks resolve from the same
directory, including the relative imports Vite emits for code splitting and
`import()` calls for lazy chunks, so a production build with multiple chunks
runs as-is. Modules see `import.meta.url`: `kaplay-native://game/<path>` for
bundled files and `file://` for `--script` entries. The host parses
`index.html` into the shim document before the game runs, so
`document.querySelector`, `classList`, and the canvas attributes in the page
behave as they do on the web.

## Bundling from source

Games without Vite plugin dependencies can be inlined into one script with the
esbuild helper:

```sh
node integrations/vite-game/build.mjs --root path/to/game/client --out build/game/game.js
kaplay-native-host --script build/game/game.js --dir path/to/game/client/public --window 1280x720
```

The bundle inlines the game and KAPLAY into one script, so no module loader is
involved. Assets keep loading from `--dir` (or a `.pak`), which means no URL
rewriting is needed. Vite virtual modules are stubbed (`virtual:pwa-register`,
atlas manifests, devtools); use `--virtual name=file` and `--define K=value` to
override them per project.

This path does not run Vite plugin transforms, so games that depend on them
(for example a sprite-atlas plugin that rewrites `loadSprite` calls) should
build with their own Vite config and use the `--web-dist` workflow above.

Games that talk to a server over HTTP or WebSockets need two flags: `--allow-network`
enables the transports and `--url` sets the page URL games see as
`window.location`, which is how relative requests and SDK endpoints resolve:

```sh
kaplay-native-host \
  --dir path/to/game/dist \
  --allow-network \
  --url "http://127.0.0.1:5173/?debug&instantLogin"
```

Relative requests check bundled assets first and only then fall back to the
page origin, so sprites and fonts keep loading from `--dir` while API calls go
to the server. With the example above, a Colyseus client pointed at `/colyseus`
connects to `ws://127.0.0.1:5173/colyseus`, exactly like in a browser served by
the Vite dev server.

## Audio

Sound plays through a native software mixer. The shim implements the Web Audio
surface KAPLAY uses (`AudioContext`, `AudioBuffer`, `AudioBufferSourceNode`,
`GainNode`, `StereoPannerNode`, `MediaElementAudioSourceNode`, `AudioParam`)
and `HTMLAudioElement` for `playMusic`, all routed through the same engine:

- Decoding: WAV, MP3, FLAC, Ogg Vorbis, and Opus, done on background threads.
- Output: ALSA on Linux, WASAPI on Windows, CoreAudio on macOS. If no device is
  available the game keeps running silently and `currentTime` still advances, so
  sounds still start and end.
- Pass `--no-audio` to skip opening a device entirely (used in CI).

## Performance

The runtime is only as fast as the build you run. For real numbers, build both
the game and the host in release mode, then run the game's production output:

```sh
cd path/to/game/client && pnpm build
cd path/to/kaplay-native && cargo build --release
node cli/src/index.mjs run --web-dist path/to/game/client/dist --no-vsync --stats
```

- `cargo build --release` picks the `release` profile (`lto = "thin"`,
  `codegen-units = 1`); `cargo build` alone produces a debug host whose frame
  time says nothing about shipping performance. The CLI prefers
  `target/release/kaplay-native-host` when both exist.
- Run `dist`, not a dev server or a source bundle. `vite build` minifies and
  substitutes `import.meta.env.DEV`, which is how games turn their own debug
  mode off.
- The host logs `renderer: <GPU> (<vendor>, OpenGL <version>)` at startup. If
  it says `llvmpipe` or `softpipe`, the GPU is not being used; CI forces
  software rendering with `LIBGL_ALWAYS_SOFTWARE`, but nothing does that by
  default. On hybrid laptops, select the discrete GPU with `DRI_PRIME=1`
  (Mesa) or `__NV_PRIME_RENDER_OFFLOAD=1 __GLX_VENDOR_LIBRARY_NAME=nvidia`
  (NVIDIA), and prefer `--x11` when the compositor is Wayland.
- Vertical sync is on by default, which caps the frame rate at the monitor
  refresh. `--no-vsync` uncaps it to show how much headroom the frame loop
  actually has. Some Wayland compositors ignore the request; `--x11` is not.
- `--stats` logs a line per second (fps and average frame time). Every run
  logs a summary on exit: total frames, fps, and average, p50, and p99 frame
  times.

## Steam

Steam support never bundles or links the Steamworks redistributables. The host
loads `steam_api64.dll` / `libsteam_api.so` / `libsteam_api.dylib` at runtime
from the game directory (or `KAPLAY_NATIVE_STEAM_LIB`) and resolves only the
flat API symbols it needs. If the library or the Steam client is missing, games
run normally and `kaplayNative.steam.available()` reports `false`.

Games call the platform API from JavaScript:

```js
if (kaplayNative.steam.available()) {
    kaplayNative.steam.unlockAchievement("FIRST_WIN");
    kaplayNative.steam.writeCloud("save.json", JSON.stringify(save));
}
```

## JavaScript engines

The host ships with QuickJS and can also be built with a V8 backend. Both
engines evaluate the same shim and the same game, and both implement the full
`__kapNative` bridge, so games do not change. Release hosts are built per
engine, so a QuickJS game never carries the ~64 MB static V8 binary. Pick the
engine at build time with Cargo features and at runtime with `--js-engine`:

```sh
cargo build -p kaplay-native-host --features v8
node cli/src/index.mjs run ./my-game --js-engine v8
```

`--engines` lists what a host binary was built with. A default build stays
QuickJS-only, and the CLI downloads the matching host flavor (`-v8`) when
`--js-engine v8` is passed. V8 pulls in a prebuilt static V8 binary. The
language surface, watchdog behavior, and parity checks are covered in
[docs/engines.md](docs/engines.md).

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cargo test -p kaplay-native-host --no-default-features --features v8
node --test shim/test/*.test.js
```

Set `KAPLAY_NATIVE_WATCHDOG=1` to make the runtime interrupt JavaScript that
stalls a phase for more than four seconds. The interrupt is reported with the
phase name and, because job exceptions are logged by the raw QuickJS pump, the
JavaScript stack that was running.

## License

MIT
