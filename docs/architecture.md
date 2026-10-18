# Architecture

## Shape of the runtime

KAPLAY Native is a drop-in host, not a fork. The repository contains seven
crates and one JavaScript shim:

| Component | Role |
| --- | --- |
| `crates/host` | winit window, glutin GL context, JavaScript engine (QuickJS or V8), frame loop, bridge |
| `crates/gfx` | `WebGLRenderingContext` over `glow`: object handles, offscreen default framebuffer, `present()` |
| `crates/canvas2d` | `CanvasRenderingContext2D` over tiny-skia: text, images, pixel ops |
| `crates/pak` | asset sources: directories and `.pak` (zip) archives |
| `crates/net` | HTTP and WebSocket transports on background threads |
| `crates/audio` | software mixer, audio decoding, and platform output |
| `crates/steam` | dynamic Steamworks flat API loading |
| `shim/src/shim.js` | browser semantics: DOM, events, canvas, WebGL wrapper, fetch, storage |

The game never imports a native module. The host installs `__kapNative` (a set
of nested function objects keyed by numeric handles), evaluates the shim, then
evaluates the game and the KAPLAY bundle unmodified.

## Boot

Startup runs in five steps:

1. The host installs `__kapNative` and evaluates the shim as a classic script.
2. `index.html`, when present, is parsed into the shim document, so the game
   sees the page's `head`, `body`, canvas, attributes, and title.
3. `--preload` files (the KAPLAY bundle, for example) are evaluated as classic
   scripts, so libraries that export globals keep working. A preload that does
   not parse as a classic script is evaluated as an ES module instead, and its
   named exports are copied onto the global object, so ESM-only bundles still
   provide globals.
4. The entry is resolved: `--script` if given, otherwise the module script
   referenced by `index.html` in the asset root, otherwise the first of
   `main.js`, `game/main.js`, `index.js` that exists.
5. The entry is evaluated as an ES module, matching
   `<script type="module">` in a browser. Top-level declarations stay
   module-scoped, which prevents game functions from colliding with the
   globals KAPLAY exports onto `window` (`UP`, `LEFT`, `RED`, and so on).
   Static `import` statements and dynamic `import()` calls load from the game
   files: relative specifiers resolve against the importing module, and root
   specifiers resolve against the asset root. A specifier that cannot be
   resolved fails evaluation with the offending specifier and importer in the
   message.

After every evaluation the host pumps the microtask queue. The pump runs each
job through `JS_ExecutePendingJob` directly instead of rquickjs's wrapper,
which swallows exceptions: a failed job is logged with its message and stack
and the remaining jobs still run.

## The bridge

`__kapNative` is the only native surface. Each namespace is a function object:

- `env` - version, platform, game name, `now()`, `log()`, `quit()`
- `window` - size, title, fullscreen, cursor
- `fs` - asset reads (`size`, `exists`, `read(path, out)`) and downloads
  (`download(name, bytes) -> saved path`)
- `storage` - `localStorage` persistence (`load`, `save`)
- `image` - decode from bytes or data URLs, dimensions
- `font` - register font bytes, query families
- `canvas` - Canvas 2D operations against a native surface id
- `gl` - WebGL 1 operations against a GL state handle table
- `audio` - buffers, voices, mixer parameters, decode jobs, and playback clock
- `steam` - Steamworks user, achievements, stats, cloud

Handles are `u32`; `-1` or a missing key means `null` on the JS side. Uploads
pass caller-owned typed arrays so the shim never copies pixels: the host reads
the typed array through the engine's backing store and writes into it directly.

`crates/host/src/js/` owns the engine choice and keeps the rest of the host
engine-agnostic. `JsRuntime` wraps one of the compiled-in backends and exposes
`eval_preload`, `eval_module`, `drain_jobs`, and `call_global`; `app.rs` calls
only those. The engine-specific notes follow:

- QuickJS (`js/quickjs.rs`) owns a `Runtime`/`Context` pair. Native functions
  capture an `Rc<HostState>` per closure and never capture `Ctx`, which avoids
  QuickJS reference cycles (capturing `Ctx` in a closure ends in a
  `JS_FreeRuntime` assertion because the closure then owns a reference to its
  own realm). Closures accept at most seven parameters because `rquickjs` only
  implements `FromParams` for tuples up to arity seven. That constraint is why
  `texImage2D` drops the `level` argument (always zero) and infers
  `texSubImage2D` heights from the byte length.
- V8 (`js/v8_engine.rs`) owns one `OwnedIsolate` on the main thread. A pointer
  to `HostState` is attached as `Function` data, and every callback recovers it
  from the callback info, so no closure captures Rust state at all. Evaluations
  run inside a context scope with the explicit microtasks policy, and the
  isolate is initialized once per process behind a `Once`.

## The frame loop

1. The host pumps winit events and forwards them to `__kapNative_dispatch`.
2. It drains network events and calls the `__kapNative_onFetch*` /
   `__kapNative_onSocket*` shim callbacks, then drains the job queue again.
3. Once per frame it calls `__kapNative_onFrame(now)`, which runs timers and
   animation frame callbacks in the shim.
4. The game calls into WebGL and Canvas 2D, which mutate the native state.
5. KAPLAY renders into its own framebuffer and finally blits it to the
   "default" framebuffer.
6. The host presents that framebuffer to the window, aspect-fit with black
   bars, and swaps buffers while the window framebuffer is still bound, which
   macOS's `flushBuffer` requires.

Neither engine pumps jobs automatically in the host's frame loop model, so
after every evaluation and frame the host drains pending work: QuickJS runs
`JS_ExecutePendingJob` until the queue is empty, and V8 runs
`perform_microtask_checkpoint()` with the explicit microtasks policy. Without
this, promise continuations never run and asset loading stalls at its first
`await`.

## Network

The `kaplay-native-net` crate owns every socket. The host thread never blocks on
I/O:

- a small pool of worker threads performs HTTP requests with `ureq`, sharing
  one agent so connections and TLS sessions are reused;
- each WebSocket gets its own thread with a 20 ms read timeout, so outgoing
  frames are written promptly while incoming frames are polled and forwarded;
- workers report `NetworkEvent`s through a channel that the frame loop drains.

Response bodies stay in Rust until the shim asks for them. The host calls a
callback with the body length, the shim allocates a `Uint8Array`, and a bridge
function copies the bytes in and frees the Rust buffer. That keeps native
functions from creating JavaScript values, which QuickJS's GC forbids for
long-lived closures.

TLS uses `rustls` with the `ring` provider (no OpenSSL dependency), and the
process installs the default crypto provider before any socket starts. Requests
and sockets are created only when `--allow-network` is passed; without it the
shim rejects immediately with a `TypeError`.

## Audio

`kaplay-native-audio` decodes and mixes every sound:

- **Decoding** runs on a bounded pool of worker threads. Files are identified by
  content: Ogg Opus streams go to `opus-pure`, and everything else (WAV, MP3,
  FLAC, Ogg Vorbis) goes to `symphonia`. Decoded samples are stored as
  interleaved `f32` in the buffer registry. The shim gets a callback when the
  decode finishes and pulls buffer metadata through `audio.bufferInfo`.
- **Mixing** runs on the output thread. A voice is a buffer plus a position,
  playback rate, gain, pan, loop flag, and optional start and stop frames. The
  mixer renders stereo blocks with linear interpolation for resampling and a
  constant-power pan law, and reports finished voices as events so `onended`
  fires.
- **Output** is ALSA (loaded dynamically, so no build-time dependency) on Linux,
  WASAPI through `cpal` on Windows, and CoreAudio through `cpal` on macOS. The
  output format is negotiated once; if the device cannot be opened, the host
  logs a warning and switches to silent mode, where the mixer clock is caught
  up to wall time from the frame loop instead of a render thread, so game logic
  is identical with or without hardware.
- **The clock** is the number of frames the mixer has rendered, exposed to the
  shim as `AudioContext.currentTime`. Parameter ramps are evaluated in the shim
  on every frame and pushed to native voices, which keeps scheduling logic in
  one place.

Music takes the `HTMLAudioElement` path: the shim fetches the file through the
normal asset resolution, decodes it through the same native pipeline, and
represents playback as a single looping or one-shot voice. `playMusic`'s
`connect`/`disconnect` calls re-evaluate the graph gain on the fly.

Oscillators and biquad filters render their note to PCM in the shim before the
voice starts: a deferred task builds the waveform and runs the RBJ biquad
recurrence over it, then the buffer goes through the same upload and play path.
The native mixer consumes buffers rather than live graphs, so notes are capped
at 30 seconds.

Video takes a parallel path. `kaplay-native-video` demuxes an MP4 with
`symphonia`'s ISO-MP4 reader and decodes the H.264 track with
`rusty_h264-decoder`, both pure Rust, so there is no C dependency and no
assembly build step. `Video::open` probes the container, requires an H.264
track, converts `avcC` to Annex-B parameter sets, and rewrites the
length-prefixed samples as Annex-B access units.

Decoding runs on a worker thread per open video, owned by the `Video` handle,
so the frame thread never probes, seeks, or decodes. The worker keeps a
decoded-frame window of up to 32 frames and 64 MiB around the playhead, decodes
forward while the window has room and stays inside the lookahead, and re-probes
the container when the shim seeks backward or jumps ahead. The shim's
`HTMLVideoElement` owns the clock, asks `frame_at(seconds)` for the newest
decoded frame at or before that time, and reports `readyState` from a
`frame_ready` query, which also nudges the worker toward a pending seek.
`gl.texImage2D` with a video source uploads that frame as an RGBA texture.
Audio from the same file is handed to the audio decoder (read from the asset
when the source is bundled), so the video's sound plays through the regular
mixer as a media element voice.

## The default framebuffer

WebGL's default framebuffer is a real offscreen framebuffer
(`DEPTH_STENCIL` renderbuffer included), not the window backbuffer. KAPLAY
binds "null" framebuffers constantly, reads pixels from them, and draws its
own framebuffer into them, so the model has to match browser behavior.

`present()` draws that offscreen framebuffer to the window with a fullscreen
quad and preserves the state a browser would preserve:

- the clear color
- the viewport
- the current program
- the active texture unit and the `TEXTURE_2D` binding
- blend, depth, stencil, and scissor enables
- vertex attribute 0's pointer/enable configuration and the `ARRAY_BUFFER`
  binding

The last two matter most. KAPLAY caches its vertex format and only re-issues
`vertexAttribPointer` when the format changes; a `present()` that leaves the
blit quad's buffer bound poisons the cache and every subsequent frame renders
nothing. The same applies to `BLEND`, which KAPLAY enables once at startup and
only calls `blendFuncSeparate` afterwards. Any new `present()` work must save
and restore everything it touches.

GLSL handling lives in `crates/gfx/src/shader.rs`: GLSL ES 1.00 sources are
translated to desktop GLSL by stripping `precision` statements and dropping
`#version 100`. `#version 300 es` is rejected.

## Canvas 2D and fonts

`crates/canvas2d` implements the 2D context on tiny-skia. Text is laid out with
selected fonts from `fontdb` (plus custom faces registered by the game) and
outlined with `ttf-parser`; a bundled DejaVu Sans Mono is the default family so
`monospace` always resolves. Images cross the boundary as raw RGBA: canvas
drawing and GL uploads read the same surfaces, so no JS-side `ImageData`
copies are needed.

Premultiplication and vertical flips follow `pixelStorei` state, tracked in
`crates/gfx/src/pixels.rs`.

## Assets

`crates/pak` reads a directory or a `.pak` zip, including one embedded in the
host executable by `kaplay-native pack`. Paths are normalized:
query strings and hashes are stripped, `..` is rejected, and resolved paths
must stay inside the asset root. Asset reads are synchronous on the JS side
(`fetch` and `Image` perform the read, then resolve the promise, matching
browser behavior closely enough for KAPLAY's loaders).

## Determinism and testing

Everything except presentation is testable headlessly: tests cover the pixel
helpers, shader translation, font parsing, color parsing, archive reads, and
the shim's browser semantics against a native stub. The host can run for a
fixed number of frames and write a screenshot, which makes visual regressions
checkable in CI with a software GL driver. CI also builds the V8 backend
separately and renders the smoke example with both engines, comparing the raw
frames byte for byte, so the two backends cannot drift apart unnoticed.
