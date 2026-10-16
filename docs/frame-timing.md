# Frame timing

Whether the V8 backend has a frame-time problem, how it was measured, and what
came out of it. Short answer: no V8-specific regression against QuickJS was
found, but the exercise turned up two real frame-time costs and one of them was
fixed.

Measured on Linux x64 (AMD Renoir iGPU, Mesa 26.1) with release hosts, headless,
uncapped (`--no-vsync`), no audio. The reference scenarios live in
`benchmarks/frame-stress`; the KAPLAY numbers come from that suite and the
packed Frax game. The stress server is `node scripts/stress-frames.mjs run`.

## Method

The existing `--bench-json` summary only reported avg/p50/p99, which hides the
frames players actually notice. Two host additions make spikes visible:

- `--bench-json` now reports `min`, `p90`, `p95`, `p999`, `max` and counts of
  frames over 16.7, 33.3, 50 and 100 ms.
- `--frame-trace <path>` writes every frame's total time and its `js`, `drain`,
  `network`, `audio` and `present` phases, so the worst frames can be attributed
  to a phase instead of guessed at.

The orchestrator drops the first 100 frames (JIT and context warm-up) and the
JS summary frame (a `console.log` at 3000 frames is a hitch if you count it).

## No V8 regression in steady state

3000 measured frames per scenario, lower is better, `p50 / p99 / max` in ms:

| Scenario | QuickJS | V8 | QuickJS >16.7 ms | V8 >16.7 ms |
| --- | --- | --- | --- | --- |
| `idle` | 0.17 / 0.60 / 2.1 | 0.16 / 0.53 / 2.0 | 0 | 0 |
| `alloc` | 1.60 / 3.12 / 4.2 | 0.23 / 0.98 / 2.0 | 0 | 0 |
| `retain` | 1.60 / 2.72 / 71.3 | 0.28 / 2.74 / 40.0 | 14 | 4 |
| `churn` | 1.68 / 2.54 / 3.5 | 0.23 / 0.76 / 2.2 | 0 | 0 |
| `burst` | 1.67 / 16.69 / 22.7 | 0.26 / 2.08 / 7.5 | 30 | 0 |
| `strings` | 1.35 / 2.15 / 3.4 | 0.33 / 0.95 / 2.5 | 0 | 0 |
| `timers` | 3.04 / 4.70 / 5.5 | 0.43 / 7.12 / 12.5 | 0 | 0 |
| `raf` | 1.06 / 1.84 / 2.4 | 0.21 / 0.74 / 1.8 | 0 | 0 |
| `microtasks` | 5.54 / 7.84 / 9.5 | 0.40 / 1.13 / 2.3 | 0 | 0 |
| `json` | 7.72 / 10.92 / 11.9 | 1.59 / 2.25 / 2.9 | 0 | 0 |
| `canvas` | 2.47 / 3.58 / 4.3 | 2.48 / 3.73 / 4.0 | 0 | 0 |
| `codegen` | 0.34 / 0.75 / 1.8 | 0.18 / 0.52 / 1.5 | 0 | 0 |
| `gc` | 1.83 / 3.54 / 4.7 | 0.22 / 2.21 / 2.8 | 0 | 0 |
| `mix` | 9.69 / 13.38 / 16.4 | 1.49 / 4.46 / 6.7 | 0 | 0 |

V8 wins or ties every scenario. The KAPLAY suite agrees: `draw` p50 3.04 ms on
V8 against 18.75 ms on QuickJS, `text` 8.51 ms against 157.77 ms, `collide`
14.26 ms against 487 ms (see the KAPLAY performance findings). Packed Frax
averages 0.26 ms p99 2.24 ms on V8 against 0.43 ms p99 7.69 ms on QuickJS,
excluding warm-up.

## Where the remaining frame-time tails come from

**V8 GC under allocation pressure.** `retain` (2000 objects/frame, live set
dropped every 240 frames) produced 4 frames over 16.7 ms on V8 and 14 on
QuickJS; worst frame 40.0 ms against 71.3 ms. `--trace-gc` shows the V8 tail is
incremental mark-compact: a 8-9 ms finalize pause at a 155-195 MB heap plus
1-3 ms of marking steps spread across 30-70 frames. Because marking steps run
inside allocations, a 30 ms frame can contain only a 9 ms atomic pause. This is
the cost of a GC'd runtime under sustained allocation; QuickJS pays more for
the same workload. It is not fixable from the embedder side without trading
throughput (semi-space and heap-size flag experiments moved the tail by less
than the run-to-run variance; running V8 idle tasks between frames showed no
gain either).

**Startup is not steady state.** Frax spends about 3 s inside the first frame's
`drain` phase on both engines: the game's async boot runs in the first
microtask checkpoint after module evaluation. The `0.6 fps (1500 ms over 2
frames)` line the host logs at launch is the first shader/context compile plus
that boot, not a frame-loop problem.

## Fix: the shim timer path

The one flag raised by the first full run was `timers` on V8 (p99 9.28 ms, max
15.8 ms). Attribution against an isolated 1500-timer-per-frame probe showed the
cost was in the shim, not the engine:

- `setTimeout`/`setInterval` called `native.env.now()` for every timer, a
  bridge crossing per timer even when the time is already known for the frame.
- `tickTimers` allocated a new array every frame, sorted every due timer even
  when they were already in order, and copied an empty `arguments` slice per
  timer.

The shim now caches the frame timestamp while `__kapNative_onFrame` runs, uses
it for timers scheduled inside a frame, reuses the due list, checks whether it
is already ordered before sorting, and shares one empty args array. Timers
scheduled outside a frame (event handlers, module load) still read the native
clock.

Timer probe (1500 timers/frame, 2000 frames, p50/p99/max in ms):

| Probe | QuickJS before | QuickJS after | V8 before | V8 after |
| --- | --- | --- | --- | --- |
| schedule + clear | 4.27 / 5.68 / 6.3 | 1.99 / 3.00 / 3.9 | 2.46 / 3.62 / 4.8 | 0.36 / 1.63 / 2.4 |
| schedule + run | 6.45 / 8.45 / 9.2 | 3.17 / 5.14 / 5.6 | 2.56 / 8.73 / 13.6 | 0.44 / 7.16 / 13.2 |
| nested timers | 34.04 / 42.06 / 49.7 | 14.19 / 18.54 / 22.0 | 12.34 / 25.70 / 27.6 | 2.11 / 11.13 / 13.4 |

In the full bed the `timers` scenario went from 6.76/9.41/10.7 to
3.04/4.70/5.5 (p50/p99/max) on QuickJS and from 2.48/9.28/15.8 to
0.43/7.12/12.5 on V8; `mix` dropped from 13.63/18.10/21.5 to 9.69/13.38/16.4
on QuickJS and 3.51/7.77/11.8 to 1.49/4.46/6.7 on V8. KAPLAY's own scenarios
are unchanged inside session noise, as expected: KAPLAY drives its own loop and
does not schedule DOM timers per frame.

## Reproduce

```sh
node scripts/stress-frames.mjs run --engine both --frames 3000
node scripts/stress-frames.mjs run --engine v8 --scenario retain --frames 5000 --verbose
```

GC attribution on V8:

```sh
KAPLAY_NATIVE_V8_FLAGS="--trace-gc" \
  target/release/kaplay-native-host-v8 --script benchmarks/frame-stress/main.js \
  --dir benchmarks/frame-stress --window 640x360 --frames 5100 --no-vsync \
  --no-audio --headless --x11 --frame-trace /tmp/retain.json \
  --url 'http://x/?scenario=retain&count=2000&frames=5000&warmup=100'
```
