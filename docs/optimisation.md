# Optimisation notes

Measured on Linux x64 with `cargo build --release -p kaplay-native-host`
(`lto = "thin"`, `codegen-units = 1`, `strip = true`) and
`cargo bloat --release -p kaplay-native-host --crates`.

## Bundle size

The release host used to be built with `--features v8`, which compiles V8 and
QuickJS into one binary. V8 and its ICU data add ~48 MB to the stripped
binary, so every QuickJS game paid for it. Release now ships an engine-specific
host per platform and the CLI downloads the matching flavor (`-v8` for
`--js-engine v8`).

| Host | Before | Engine split | Link-time folding | Slim V8 |
| --- | --- | --- | --- | --- |
| QuickJS (default) | 13.8 MB | 11.8 MB | 10.6 MB | n/a |
| V8 | 60.1 MB | 58.1 MB | 51.8 MB | 30.4 MB |
| QuickJS + V8 (old single artifact) | 61.7 MB | n/a | n/a | n/a |

Link-time folding is `panic = "abort"` plus LLD's `--icf=all`; the slim V8
build drops i18n, WebAssembly and Temporal. Both are described below.

What the 11.8 MB QuickJS binary spends its text section on, according to
`cargo bloat`:

| Component | Text |
| --- | --- |
| `std` + unknown/glue | ~2.1 MB |
| `winit` + `wayland`/`x11` | ~800 KB |
| `rustls` + `ring` + `ureq` + `tungstenite` | ~950 KB |
| `tiny_skia` + `fontdb`/`ttf-parser` | ~600 KB |
| `symphonia` + `opus_pure` (incl. `rustfft` ~700 KB) | ~1.3 MB |
| QuickJS (`rquickjs_core` + sys) | ~330 KB |
| `clap_builder` (CLI parser) | ~230 KB |
| image decoding (`image` + `zune-jpeg` + `png`) | ~250 KB |
| `kaplay-native-*` crates | ~380 KB |

Cargo changes on this branch:

- `strip = true` in the release profile. The unstripped binary carries ~2.7 MB
  of symbol/debug sections that never ship.
- `zip` only enables `deflate-flate2`. The pak format is written by
  `cli/src/zip.mjs` and only ever uses store (method 0) and deflate (method 8),
  so bzip2, zstd, xz/lzma, ppmd, deflate64, aes/sha1/hmac and zopfli were dead
  weight.
- `rquickjs` no longer enables `full` (chrono, dyn-load, module loader,
  either, indexmap, phf) and drops the derive macros; the bridge only needs
  `std` and `array-buffer`.
- `env_logger` no longer pulls `regex`/`regex-automata`/`aho-corasick`
  (~600 KB) for log filtering.
- `symphonia` drops `all-meta` and `adpcm`. `isomp4` and `aac` come back with
  video support, which demuxes MP4 and plays the AAC track of a cutscene.

### Link-time folding

`[profile.release]` sets `panic = "abort"`, and Linux hosts are linked with
LLD and `--icf=all`:

```sh
RUSTFLAGS="-C force-unwind-tables=no -C link-arg=-fuse-ld=lld -C link-arg=-Wl,--icf=all" \
  cargo build --release -p kaplay-native-host
```

Aborting panics delete landing pads and exception tables (~0.8 MB). ICF folds
functions with identical machine code; V8's own release build enables it
(`build/config/compiler/BUILD.gn`), so it is compatible with the prebuilt
archive. On the stripped Linux x64 host this takes V8 from 58.1 MB to
51.8 MB and QuickJS from 11.8 MB to 10.6 MB. `--gc-sections` is already passed
by rustc and changes nothing. The tradeoff is that a Rust panic aborts the
process instead of unwinding, so `catch_unwind` users such as `rquickjs` no
longer convert panics into JS errors. The flags are applied in `ci.yml` and
`release.yml`; Linux jobs install `lld` for them. macOS keeps unwinding tables
because its linker has no `--icf` and Objective-C exceptions can cross Rust
frames.

### Inside V8

The `v8` crate exposes only `use_custom_libcxx` (default on, statically links
libc++), `simdutf`, `v8_enable_pointer_compression`, `v8_enable_sandbox` and
`v8_enable_v8_checks`. `simdutf` and `v8_enable_v8_checks` only add code; the
compression/sandbox flags are memory and security knobs that are off in this
build. None of them shrink the JS feature set.

The unmodified prebuilt archive is what CI downloads today (`V8_FROM_SOURCE`
is not set). A linker map of the 51.8 MB host after ICF shows where its code
and read-only data go:

| Component | `.text` | `.rodata` | Total |
| --- | --- | --- | --- |
| V8 core (runtime, objects, parser, regexp, builtins) | 12.1 MB | 1.3 MB | 13.3 MB |
| ICU data (`icudtl_dat.o`) | — | 10.4 MB | 10.4 MB |
| TurboFan / Turboshaft | 4.5 MB | 0.1 MB | 4.6 MB |
| Embedded builtins snapshot | 2.2 MB | 0.1 MB | 2.3 MB |
| Maglev | 2.2 MB | 0.1 MB | 2.3 MB |
| WebAssembly | 2.2 MB | — | 2.2 MB |
| Rust crates + host | 6.7 MB | 1.2 MB | 7.9 MB |

The single biggest removable piece is not code: `icudtl_dat.o` embeds 10.4 MB
of ICU locale data. QuickJS has no `Intl`, so dropping i18n at build time also
improves cross-engine parity. `scripts/build-slim-v8.sh` builds V8 from the
vendored sources with

```sh
V8_FROM_SOURCE=1 GN_ARGS="v8_enable_i18n_support=false v8_enable_webassembly=false v8_enable_temporal_support=false"
```

and produces a 30.4 MB host (23.6 MB `.text`, 2.9 MB `.rodata`): 21.4 MB less
than the ICF build of the prebuilt archive and 28 MB less than the original.
`typeof Intl`, `typeof WebAssembly` and `typeof Temporal` now return
`undefined` on both engines, and `(123456.789).toLocaleString()` matches
QuickJS. Smoke frames are byte-identical, the bridge example logs the same
output, and the Frax game runs on V8 15.2.124.1-rusty. Temporal is dropped
because V8 bakes an ICU `zoneinfo64` blob into the build whenever it is
enabled, and the published crate prunes the generator script, so it cannot be
built from the crate source at all. Disabling Maglev would save ~2.3 MB more
but removes the mid-tier JIT that keeps V8 attractive.

The script supports Linux, macOS and Windows hosts, caches gn and LLVM
(libclang 21+ is required by bindgen) in `~/.cache/kaplay-native`, and applies
the same ICF link flags when `ld.lld` is available (including rustup's
toolchain copy). The `Slim V8` workflow runs it on GitHub for Linux x64, Linux
arm64, macOS arm64, macOS x64 (cross-built) and Windows x64, smoke-tests the
host against QuickJS where the runner can, and publishes
`librusty_v8_slim_<version>_<target>.a` (`.lib` on Windows) on a
`slim-v8-<version>` release. Each job records `config=<version>-<recipe>` in
the release notes and skips itself when its asset is already published for
that recipe, so plumbing fixes leave published archives alone; bump
`SLIM_V8_RECIPE` in the script when the GN args or the crate patches change.
`release.yml` downloads that asset for every platform and links the `-v8`
hosts against it with `RUSTY_V8_ARCHIVE` (the prebuilt bindings are
compatible), falling back to the prebuilt archive when the release is missing.
Any host can link a local slim archive with
`RUSTY_V8_ARCHIVE=path/to/librusty_v8.a`.

Remaining bundle-size candidates:

1. **Codec presets.** `rustfft` (700 KB) is a hard dependency of
   `symphonia-core`; a game that only uses WAV/Opus could drop all of
   symphonia with a `kaplay-native-audio` feature set. Same idea for the
   network stack (~950 KB) and `zune-jpeg`/`png` width.
2. **`clap`.** ~230 KB for ~25 flags. A hand-rolled parser or `lexopt`
   (~30 KB) would pay for itself on every shipped game.
3. **Shim minification.** `shim/src/shim.js` is 121 KB of source evaluated at
   startup. A minified ~45 KB variant would cut both bytes and parse time;
   V8 could also use a startup snapshot.
4. **`opt-level = "s"` for dependencies** (`[profile.release.package."*"]`)
   with the hot crates kept at `3`; `lto = "fat"` is worth re-measuring.
5. **Strip `.eh_frame` after linking.** It survives `strip = true` and still
   costs ~3.2 MB on the V8 host, almost all of it V8 C++. Removing it saves
   that much but deletes backtraces and C++ unwinding metadata, so it is only
   worth it if crash diagnosis does not matter.

## Threading

Today:

| Subsystem | Thread | Communication |
| --- | --- | --- |
| JavaScript engine, GL, winit, Steam | main | direct calls |
| Audio output (ALSA/CoreAudio/WASAPI) | device callback thread | `Mutex<Mixer>`, `Mutex<Vec<Event>>` |
| Audio decode | pool of `ceil(cores/2)` clamped to 1–4 | mpsc jobs, mpsc events |
| HTTP fetch | 4 workers | mpsc jobs/events, one shared `ureq::Agent` |
| WebSocket | one thread per socket | mpsc out, mpsc events |
| Canvas 2D, fonts | main | direct calls |

Constraints that keep GL and the JS engine on the main thread: OpenGL contexts
are thread-affine, and neither QuickJS nor the V8 isolate is `Send`. Everything
else is a candidate for a dedicated thread.

Changes on this branch:

- Audio decoding used `std::thread::spawn` per request, so a large game could
  create hundreds of short-lived threads during load. It now runs on a bounded
  pool.
- The silent backend no longer runs a render thread that woke up every ~10 ms.
  `Engine::advance_silent` catches the mixer clock up to wall time from the
  frame loop, which removes a thread and a wakeup source from `--no-audio`
  runs and from devices that fail to open.
- HTTP fetch workers used to build a new `ureq::Agent` for every request, which
  discards the connection pool and forces a fresh TLS handshake each time.
  Workers now share one agent, which matters for the polling traffic games
  generate.

Target shape, if more isolation is wanted: every subsystem owns a command
sender and an event receiver, and the main thread never blocks on a subsystem
lock.

- **Mixer.** The audio callback currently locks the `Mixer` mutex that the main
  thread also holds while updating voices. Replacing it with a lock-free
  command ring (main -> mixer) and an event ring (mixer -> main) removes the
  possibility that the frame thread stalls the audio device.
- **Asset I/O.** `AssetSource::read` runs on the JS thread, so a big PNG blocks
  the frame. Image decode (`image::load_from_memory`) is CPU-heavy and could
  use the same job/event pattern as audio decode, with the shim awaiting an
  event.
- **Pak loading.** `from_pak_bytes` decompresses every entry at startup. For
  large games, keep the archive and decompress on demand into a small cache.
- **Canvas 2D.** Synchronous text/rect drawing has to stay on the main thread
  because the JS calls return immediately, but `to_png`, `getImageData` and
  image decode can move to workers.

## Hot paths

- **Mixer pan law.** `pan_gains` called `cos`/`sin` once per voice per sample,
  i.e. ~1M trig calls per second with a dozen voices. The gain pair is now
  computed when a voice is created or its pan changes.
- **Asset reads.** Every `fs.read`/`image.load` cloned the whole file out of the
  archive. Entries are now `Arc<[u8]>`, so a read is a refcount bump. `fs.size`
  also stopped reading the whole file just to report its length, and directory
  mode canonicalizes the asset root once instead of twice per lookup.
- **QuickJS texture uploads.** `texImage2D`/`texSubImage2D` copied the typed
  array into a `Vec` before upload; they now borrow the array for the duration
  of the GL call.
- **Frame statistics.** `record_frame` grew a `Vec` by one entry per frame
  forever, even when `--stats` was off. It is now a 10 000-sample ring, which
  caps memory at 40 KB.
- **Shim frame loop.** `__kapNative_onFrame` copied the animation-frame queue
  with `splice` (a fresh array every frame) and walked every timer. It now
  swaps buffers and skips the timer map when it is empty.
- **Shim timers.** `setTimeout`/`setInterval` crossed the bridge for
  `native.env.now()` on every call, `tickTimers` allocated a due array and
  sorted it even when already ordered, and every timer copied an empty
  `arguments` slice. The shim now reuses the frame timestamp while a frame is
  running, reuses the due list, skips the sort when it is not needed and
  shares one empty args array. 1500 timers/frame: schedule+clear p50 2.46 ms
  → 0.36 ms on V8 and 4.27 ms → 1.99 ms on QuickJS; nested timers p50
  12.34 ms → 2.11 ms and 34.04 ms → 14.19 ms. See `docs/frame-timing.md`.

Remaining hot path candidates (numbers below are pre-optimisation Linux x64
quickjs baseline figures; current baselines live in `benchmarks/README.md`):

- `Mixer::render` was voice-major rewritten: per-voice channel slices are read
  directly instead of `read_frame` per sample (162 µs → 87 µs per 512-frame
  block at 32 voices, ~1.8x across 32-128 voices).
- `AssetSource` used to decompress every pak entry up front; it now indexes the
  central directory and decompresses on first read into a cache
  (`pak/open/frax` 11.9 ms → 3.4 ms).
- `net`'s socket worker blocked on a 20 ms read poll, so every WebSocket send
  waited for the next poll (`echo-roundtrip` 21.3 ms). It is now non-blocking,
  wakes on `polling` notifications from the send path, and sets `TCP_NODELAY`
  (21.3 ms → 0.1 ms; batch of 64 → 0.65 ms).
- `canvas2d` premultiplies a copy of the source on every `drawImage`; opaque
  sources are now borrowed without a copy (512² opaque 1.53 ms → 1.32 ms, and
  no per-draw allocation). The remaining cost is the tiny-skia bilinear blit.
- `canvas2d` text layout is rebuilt per `fillText`/`strokeText` call
  (`fill_text_long` 793 µs, `stroke_text_long` 1.7 ms) and every glyph outline
  is rebuilt per call; a glyph outline cache is the next candidate.
- `GlState::present` issues ~10 `glGet*` calls per frame, but `state/*` metrics
  put each GL call at ~0.4 µs against a 1.56 ms swap-bound `present`, so state
  tracking was dropped as not worth the risk.
- Audio decode is asynchronous already; long assets (`Forest.ogg` 43.8 ms)
  occupy a worker instead of the load path.
- Input events serialize JSON for every key/mouse event before entering the
  shim.
- The shim re-walks the Web Audio node graph for every live native voice on
  every frame (`graphParams` allocates a stack and a `Set` per voice). Graphs
  with effects that need sample processing are rendered once when the source
  starts; the tradeoff is a one-time CPU and buffer cost instead of live
  parameter updates. The analyser tap copies one mono sample per mixed frame
  into a 32k-frame ring buffer.

## How to measure

`benchmarks/README.md` documents the per-crate and integrated benchmark suites
and the committed Linux x64 baselines. Quick loop:

```sh
node scripts/bench.mjs run     --engine quickjs --only game --no-build
node scripts/bench.mjs compare --engine quickjs
```

`--bench-json` writes the frame-time percentiles and per-phase (js, drain,
network, audio, present) breakdown from an integrated run:

```sh
target/release/kaplay-native-host --pak game.pak --no-vsync --bench-json bench.json --frames 600
```

`--frame-trace <path>` writes every frame's total and phase times for spike
attribution, and `docs/frame-timing.md` documents the stress bed built on it:

```sh
node scripts/stress-frames.mjs run --engine both --frames 3000
```

Extra V8 flags can be passed for diagnosis before the isolate initialises:

```sh
KAPLAY_NATIVE_V8_FLAGS="--trace-gc" target/release/kaplay-native-host-v8 --pak game.pak --no-vsync
```

Bundle size:

```sh
cargo build --release -p kaplay-native-host
cargo bloat --release -p kaplay-native-host --crates -n 40
stat -c '%s' target/release/kaplay-native-host
```

Slim `--release` builds use the CI flags:

```sh
RUSTFLAGS="-C force-unwind-tables=no -C link-arg=-fuse-ld=lld -C link-arg=-Wl,--icf=all" \
  cargo build --release -p kaplay-native-host
```

Runtime frame cost, without the vertical-sync wait:

```sh
target/release/kaplay-native-host --pak game.pak --no-vsync --stats --frames 600
```

Cross-engine parity and packed-game smoke tests live in `ci.yml` and
`release.yml`.
