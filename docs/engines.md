# JavaScript engines

The host can evaluate the same shim and the same game with either QuickJS or
V8. Engine support is compiled in behind Cargo features, and the runtime
selects the engine at startup, so a build can offer one engine or both.

| Engine | Crate | Feature | Default | Notes |
| --- | --- | --- | --- | --- |
| QuickJS | `rquickjs` 0.14 | `quickjs` | yes | ES2020, no `Intl`, explicit job queue |
| V8 | `v8` (rusty_v8) 152 | `v8` | no | full modern JS including `Intl`, prebuilt static library |

No game-facing API changes. The shim is evaluated identically on both engines,
and every `__kapNative` namespace is implemented twice: once in
`crates/host/src/bridge/quickjs.rs` and once in
`crates/host/src/bridge/v8_engine.rs`.

## Building

QuickJS only (the default build):

```sh
cargo build -p kaplay-native-host
```

V8 only:

```sh
cargo build -p kaplay-native-host --no-default-features --features v8
```

Both engines, selected at runtime:

```sh
cargo build -p kaplay-native-host --features v8
```

The `v8` crate downloads a prebuilt static V8 for Linux, macOS, and Windows on
x64 and arm64. To build V8 from source instead, set `V8_FROM_SOURCE=1` before
`cargo build` (slow, requires a full toolchain and network access to fetch the
V8 checkout).

`scripts/build-slim-v8.sh` builds V8 from source without `Intl`,
`WebAssembly` and `Temporal`, which matches QuickJS's feature set and shrinks
the Linux x64 host from 51.8 MB to 30.4 MB. The script is Linux-only for now;
see [optimisation.md](optimisation.md#inside-v8) for measurements and
[distribution.md](distribution.md) for how packed games choose a host.

At least one engine feature must be enabled; a build with neither fails with
`enable at least one of the quickjs or v8 features`.

## Running

List the engines compiled into a host binary:

```sh
target/release/kaplay-native-host --engines
node cli/src/index.mjs engines
```

Pick an engine for a run:

```sh
target/release/kaplay-native-host --script examples/smoke/main.js --dir examples/smoke --js-engine v8
node cli/src/index.mjs run ./my-game --js-engine quickjs
```

`--js-engine` must name a compiled-in engine; a single-engine build accepts
only that engine and fails at startup when asked for the other. When both are
compiled in, the default is QuickJS.

The CLI prefers the local release host when it supports `--js-engine`, then
checks the debug host, so `cargo build --features v8` works even if an older
QuickJS-only release build exists. `--host` and `KAPLAY_NATIVE_HOST` override
that selection; to select the debug build explicitly:

```sh
node cli/src/index.mjs run --web-dist ./dist --js-engine v8 --host ./target/debug/kaplay-native-host
node cli/src/index.mjs engines --host ./target/debug/kaplay-native-host
```

`kaplay-native pack --js-engine v8` embeds the choice in a distributed game, so
a packed binary starts with V8 without any flags. It also makes the CLI use the
V8 host flavor, which is why a default QuickJS-packed game cannot be switched
to V8 with a runtime flag; see [distribution.md](distribution.md).

## Behavior differences

The bridge and the shim are engine-independent, so games behave the same on
both. The engines differ in the JavaScript language services they provide:

- `Intl` is available on the prebuilt V8 and absent on QuickJS and on slim V8
  builds. KAPLAY's `runes.ts` falls back to a code-point splitter on QuickJS,
  so text still works without locale-aware segmentation.
- The prebuilt V8 also has `WebAssembly` and `Temporal`; QuickJS and slim V8
  builds do not, so games that ship `.wasm` or use `Temporal` need the prebuilt
  archive.
- Error messages and stack traces come from different implementations. Code
  should not parse them.
- QuickJS exposes a pending-job count, so the host can warn when a frame
  queues an unreasonable number of promise jobs. V8's checkpoint does not
  report a count; the warning is QuickJS-only.
- Unhandled promise rejections are logged by both engines. QuickJS logs them
  through its rejection tracker, V8 through a promise-reject callback, and both
  include the rejection reason.

## How the abstraction works

`crates/host/src/js/` owns the engine choice:

- `js/mod.rs` declares `JsEngine`, defines `available_engines()` and
  `default_engine()` from the compiled features, and wraps the selected backend
  in `JsRuntime`.
- `js/quickjs.rs` owns the QuickJS `Runtime`/`Context` pair and pumps
  `JS_ExecutePendingJob` directly, so a failing job is logged with its message
  and stack and the remaining jobs still run.
- `js/v8_engine.rs` owns one `OwnedIsolate` on the main thread, enters a
  context scope for every evaluation, and runs
  `perform_microtask_checkpoint()` with the explicit microtasks policy.

`JsRuntime` exposes the same methods to `app.rs` on both paths:
`eval_preload`, `eval_module`, `drain_jobs`, and `call_global(name, args)`,
where arguments are a small `JsArg` enum (unsigned integer, float, boolean,
string). Event callbacks from the network and audio threads are dispatched
through `call_global`, which is why `app.rs` contains no engine-specific code.

The bridge installs the same function names into the same namespaces on both
engines. The V8 path attaches a pointer to `HostState` as `Function` data, and
each callback recovers it from the callback info; the QuickJS path captures an
`Rc<HostState>` in each closure instead.

## Watchdog

`KAPLAY_NATIVE_WATCHDOG=1` interrupts JavaScript that stalls a boot phase
(`preload`, `boot`) or frame (`frame update`, `drain`) for more than four
seconds. The mechanism differs per engine:

- QuickJS installs an interrupt handler through `Runtime::set_interrupt_handler`.
- V8 spawns a watchdog thread holding an `IsolateHandle`; it polls a shared
  deadline and calls `terminate_execution()`, which resolves to an uncatchable
  termination exception on the main thread.

Both report the stalled phase and leave the host running. Independently of
the watchdog, V8 microtask drains are always bounded by a four-second
deadline guard so a self-requeueing microtask cannot hang the frame.

## Parity checks

Engine parity is enforced by tests and CI, not by convention:

- `examples/bridge/main.js` calls every `__kapNative` namespace and logs one
  deterministic `BRIDGE {...}` line. The render CI job runs it with both
  engines and diffs the lines.
- The render CI job runs `examples/smoke` with both engines and compares the
  raw RGBA frames byte for byte, then checks that the frame is non-trivial.
- `cargo test -p kaplay-native-host --no-default-features --features v8` runs
  the V8-specific unit tests, and the default `cargo test` keeps covering the
  QuickJS path.

To run the parity checks locally:

```sh
cargo build --release -p kaplay-native-host --features v8
target/release/kaplay-native-host --script examples/smoke/main.js --dir examples/smoke \
  --window 320x240 --frames 5 --no-steam --no-audio --screenshot-raw quickjs.rgba
target/release/kaplay-native-host --script examples/smoke/main.js --dir examples/smoke \
  --window 320x240 --frames 5 --no-steam --no-audio --js-engine v8 --screenshot-raw v8.rgba
cmp quickjs.rgba v8.rgba
```
