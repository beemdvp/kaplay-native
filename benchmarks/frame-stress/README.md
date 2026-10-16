# Frame stress

A stress bed for frame times: scenarios push allocation, timers, animation
frames, microtasks, JSON and canvas work through the shim and the JS engine,
and the host reports every frame rather than only an average. It exists to
answer "is this a frame-time problem, which engine, and which phase?" before
blaming a game.

The scenarios are engine-agnostic on purpose: no KAPLAY, no assets, no game
logic. They isolate the host frame loop, the shim and the JS engine.

## Scenarios

| Scenario | Default count | What it stresses |
| --- | --- | --- |
| `idle` | 0 | the bare loop, frame start/end and present |
| `alloc` | 2000 | short-lived objects with a sliding window (young-gen GC) |
| `retain` | 2000 | a growing live set dropped every 240 frames (major GC) |
| `churn` | 2000 | replacing one large object graph per frame |
| `burst` | 2000 | 8x allocation on 20 of every 120 frames |
| `strings` | 4000 | string building and concatenation |
| `timers` | 1500 | `setTimeout(fn, 0)` scheduled every frame |
| `raf` | 1500 | `requestAnimationFrame` callbacks queued every frame |
| `microtasks` | 4000 | resolved promise chains, drained after each frame |
| `json` | 2000 | `JSON.parse`/`JSON.stringify` round trips |
| `canvas` | 16 | `fillText` through the canvas2d bridge |
| `codegen` | 32 | `new Function` compilation inside frames |
| `gc` | 2000 | allocation plus a forced `gc()` every 30 frames (V8 only) |
| `mix` | 1500 | allocation + JSON + timers together |

## Usage

```sh
node scripts/stress-frames.mjs run
node scripts/stress-frames.mjs run --engine v8 --scenario retain --frames 5000
node scripts/stress-frames.mjs run --no-build --count 4000 --json /tmp/stress.json
node scripts/stress-frames.mjs run --vsync --frames 600 --scenario retain --count 20000
```

The orchestrator builds (or reuses) `target/release/kaplay-native-host-<engine>`
with the same convention as `scripts/bench.mjs`, runs every scenario on both
engines, drops the first `--warmup` frames, and prints p50/p99/p99.9/max, the
dispersion (`sd`, coefficient of variation `cv%`), frames over 1.5x the median,
missed refresh ticks, over-budget counts, and the worst frames with their
dominant phase. It always passes `--headless`; run a host by hand if you want a
visible window.

By default it passes `--no-vsync` to measure raw frame cost. `--vsync` leaves
vsync on and reports `missed`, the number of display refresh ticks the run was
late by (computed against the measured median interval). A hidden window still
syncs to the display, so this works headless.

The host gains two flags for this:

- `--frame-trace <path>` writes every frame's total ms and the `js`, `drain`,
  `network`, `audio` and `present` phases as JSON (last 10 000 frames). This is
  what makes spikes attributable instead of just visible.
- `KAPLAY_NATIVE_V8_FLAGS="--trace-gc"` passes extra flags to V8 before it
  initialises, e.g. to line GC pauses up with the frames that stalled.

`--bench-json` on the same run carries the summary statistics in JSON,
including the over-budget counts added here.

## Reading a run

- The first frames are JIT and shader/context warm-up. Every summary excludes
  `--warmup` frames; the orchestrator also slices off the JS summary frame, so
  the `console.log` that ends a scenario does not count as a hitch.
- Consistency shows up in `sd`/`cv%` and in `1.5x` (frames over 1.5x the
  median). Under `--vsync`, `missed` is the number of refresh deadlines the
  run overran; a smooth 60 Hz game should have none.
- Absolute numbers drift between sessions on a laptop (CPU state, thermals).
  Compare engines within one run and record a fresh baseline for a change.
- A worst frame whose dominant phase is `present` is GL/driver pacing; one
  whose dominant phase is `js` is the engine (allocation, GC, JIT) or the
  game's own callbacks.
- `gc` is a diagnostic, not a comparison: QuickJS has no exposed `gc()`, so
  the forced collection only happens on V8.
