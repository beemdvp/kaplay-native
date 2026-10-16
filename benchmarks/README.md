# Component benchmarks

Per-crate benchmarks plus one integrated KAPLAY run, so each crate is measured
on what it cares about and all of them are exercised together on every run.
Linux x64 is the current baseline platform.

## Layout

| Path | Role |
| --- | --- |
| `crates/pak/benches/pak.rs` | archive open, lookup, read, path normalize |
| `crates/audio/benches/audio.rs` | mixer render per voice count, voice lifecycle, decode per codec |
| `crates/canvas2d/benches/canvas2d.rs` | text, image draw, pixel ops, PNG encode |
| `crates/video/benches/video.rs` | open, sequential decode, frame copy |
| `crates/net/benches/net.rs` | loopback HTTP roundtrips, WebSocket echo |
| `crates/gfx/examples/bench.rs` | hidden-window GL: uploads, buffers, draws, state, present, readback |
| `examples/bench/main.js` | kitchen sink: GL sprites, canvas2d text/images, audio, video, HTTP, WebSocket |
| `benchmarks/kaplay/main.js` | KAPLAY game scenarios: sprites, movement, text, collision, churn, events, draw-call micros |
| `scripts/bench.mjs` | orchestrator, loopback HTTP/WS server, baseline diff |
| `scripts/bench-kaplay.mjs` | orchestrator for the KAPLAY scenarios, baseline diff |
| `benchmarks/baselines/linux-x64-<engine>.json` | committed baselines |
| `benchmarks/baselines/kaplay-linux-x64-<engine>.json` | committed KAPLAY baselines |
| `benchmarks/results/` | run output (ignored) |

The integrated scenario prints deterministic `BENCH key=value` lines. The
orchestrator diffs them between engines and against the baseline; a mismatch is
a functional regression. `BENCH_INFO` lines are timing-dependent and never
compared.

## Usage

```sh
node scripts/bench.mjs run      --engine quickjs
node scripts/bench.mjs baseline --engine quickjs
node scripts/bench.mjs compare  --engine quickjs
node scripts/bench.mjs run      --engine quickjs --quick --only game
```

`--only game` skips criterion and the GL bench. `--no-build` reuses the copied
hosts in `target/release/kaplay-native-host-<engine>`. `--verbose` prints host
output. Compare warns above 5% and fails above 10% regression per metric, with
`fps` and `gib_per_s` treated as higher-is-better. Metrics whose baseline value
is under 1 µs are listed with `*` but not gated: they bounce by 2x between runs
even on an idle machine.

The host side of a run is the `--bench-json <path>` flag: it writes frame-time
percentiles plus per-phase (js, drain, network, audio, present) averages and
p50/p99.

## KAPLAY benchmarks

The KAPLAY scenarios measure the JS engine half: the game runs on the real
KAPLAY build through the native host, so the numbers cover KAPLAY's own game
loop, ECS, draw pipeline, text, physics and event dispatch. They gate changes
to a KAPLAY checkout against a committed baseline.

```sh
node scripts/bench-kaplay.mjs run      --engine quickjs --no-build
node scripts/bench-kaplay.mjs baseline --engine both --no-build
node scripts/bench-kaplay.mjs compare  --engine quickjs --no-build
node scripts/bench-kaplay.mjs run      --engine quickjs --scenario draw
```

The bundle defaults to `../kaplay/dist/kaplay.js`; pass `--kaplay <path>` to
point at another build. Build it with `npm run build` in the KAPLAY checkout
first, since the baseline records the bundle hash and git commit. Each
scenario runs twice by default and the lower JS p50 wins; use
`--repeats <n>` to trade runtime for stability.

| Scenario | Count | What it measures |
| --- | --- | --- |
| `idle` | 0 | the loop, frame start/end and the empty draw pipeline |
| `draw` | 400 | static sprites: transform collect, sort and draw |
| `move` | 400 | sprites moved from a per-object `onUpdate` |
| `text` | 400 | static text objects: formatted text draw |
| `collide` | 200 | overlapping `area()` bodies: broad and narrow phase |
| `churn` | 400 | `add`/`destroy` of a fresh batch every other frame |
| `events` | 400 | three `onUpdate` listeners per object |
| `microquad` | 400 | `drawUVQuad` calls from one draw handler |
| `microsprite` | 400 | `drawSprite` calls from one draw handler |
| `micromatrix` | 400 | `pushTransform`/`pushTranslate`/`popTransform` |

`BENCH` lines are deterministic (`objects`, `checksum`, `fired`) and are
compared for exact equality between the baseline and the current run, and
between engines. `BENCH_INFO` timings are reported but never gated. The gate
diffs the JS phase p50 and the micro `loop_ms` timings per scenario: a warn is
5% and a failure 10%, with a 20 µs floor. `idle` and `micromatrix`, `fps`,
the frame percentiles including p99 and the `drain`, `network`, `audio` and
`present` phases are recorded and reported but never gate, because they are
diagnostic, bounce on this machine, or measure the host rather than KAPLAY.

Absolute numbers drift between sessions on this laptop. Record a baseline
back-to-back with the change being measured instead of comparing against a
stale one.

## Baseline snapshot

Linux x64, AMD Renoir iGPU, Mesa 26.1, 900 frames, 1280x720 for Frax and
960x540 for the integrated scenario. Both engines, recorded after the
optimisations below.

| Metric | quickjs | v8 |
| --- | --- | --- |
| integrated frame p50 / p99 | 2.61 ms / 3.90 ms | 1.90 ms / 2.91 ms |
| integrated fps | 340 | 471 |
| integrated js phase p50 | 2.25 ms | 1.62 ms |
| Frax fps, frame p50 / p99 | 211, 0.22 ms / 9.70 ms | 262, 0.17 ms / 3.07 ms |
| `pak/open/frax` | 3.82 ms | 3.43 ms |
| `audio/mixer/render/32` / `/128` | 107 µs / 404 µs | 90 µs / — |
| `net/ws/echo-roundtrip` | 104 µs | 100 µs |
| `net/http/roundtrip` 4 KiB / 64 KiB | 188 µs / 232 µs | — |
| `canvas2d/draw_image` opaque / alpha 512² | 1.58 ms / 2.05 ms | — |
| `canvas2d/draw_image_from_canvas` 256² | 520 µs | — |
| `canvas2d/text/fill_text_long` | 858 µs | — |
| `video/decode-30fps` (160x120) | 6.64 ms | — |
| `gfx/texture_upload` 512² | 62 µs | — |
| `gfx/draw_arrays` 4 verts | 2.81 µs | — |
| `gfx/present` 1280x720 | 1.76 ms | — |

## Optimisations landed

Measured against the pre-optimisation quickjs baseline (`--quick` filter runs,
same machine):

- **`net` WebSocket latency.** The socket worker only checked its outbound
  queue between 20 ms read polls, so every send waited for the next poll. The
  socket is now non-blocking, registered with a `polling` poller, and the send
  path notifies the worker; `TCP_NODELAY` is set like browsers do.
  `net/ws/echo-roundtrip` 21.3 ms → 0.10 ms, batch of 64 63.1 ms → 0.69 ms.
- **`audio` mixer.** `Mixer::render` was rewritten voice-major: per-voice
  channel slices are read directly instead of `read_frame` per sample, position
  is kept in a register, and the per-frame clamp pass is skipped when nothing
  rendered. 32 voices 162 µs → 97 µs per 512-frame block, 128 voices 621 µs →
  383 µs, one voice 5.8 µs → 3.2 µs.
- **`pak` startup.** `AssetSource` used to decompress every entry at open; it
  now indexes the central directory and decompresses on first read into a
  cache. `pak/open/frax` 11.9 ms → 3.8 ms, synthetic deflate 1.24 ms → 0.19 ms.
- **`canvas2d` opaque draw.** `premultiply` copied the whole source on every
  `drawImage`; fully opaque sources are now borrowed via `Cow`. 512² opaque
  improved and no longer allocates per draw.
- **`gfx` present state queries were measured and dropped.** The 9 `glGet*`
  save/restore calls cost ~4 µs of a 1.76 ms swap-bound present, so tracking GL
  state in Rust is not worth the risk.

## Caveats

- The GL bench and the host need a display; under llvmpipe the renderer string
  is recorded but the numbers are meaningful only for relative changes within
  one machine.
- Absolute numbers drift by ~20% between sessions on this laptop (CPU state,
  fonts, thermal). Compare only against a baseline recorded back-to-back with
  the current run.
- Net benches are loopback only. TLS handshakes are not benchmarked.
- Audio benches run on the silent backend, so they measure mixer and decode
  CPU, not the device.
- Frax paces its own loop; use total fps as ground truth and the phase p50s as
  per-active-frame cost.
- CI is intentionally not wired to these thresholds yet.

## Remaining candidates

- `canvas2d` text: every `fillText`/`strokeText` re-parses the face, re-lays
  out the string, and rebuilds each glyph outline (`fill_text_long` 858 µs,
  `stroke_text_long` ~1.7 ms). A glyph outline cache keyed by font and glyph id
  is the next step; it changes rendering internals, so it needs a visual pass.
- `canvas2d` `to_png` (8.5 ms for 960x540) uses the default encoder settings.
- Input events serialize JSON for every key/mouse event before entering the
  shim.
- The shim re-walks the Web Audio node graph for every live voice on every
  frame (`graphParams` allocates a stack and a `Set` per voice).

## Branch notes

The PR for this work also carries the incidental fixes: `zip` now selects the
pure-Rust deflate backend explicitly (`deflate-flate2-zlib-rs`) so per-crate
builds no longer depend on `png` or `tiny-skia` enabling a flate2 backend; the
host gained `--bench-json` with per-phase timings and the CLI passes it
through; the video decode test helper now checks `frame_ready` before reading
the frame like the shim does, removing a flaky seek assertion; the compare
report sorts without `NaN` and carries the sub-microsecond gate.
