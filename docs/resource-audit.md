# Resource lifecycle and hot-path audit

Status: implemented and verified. Every item is checked off only after the code
and its tests are in place.

## Leaks

- [x] L1. Uniform and attribute location handles grow without bound.
  - Native `GlState` caches `(program, name)` lookups and removes the cached
    handles on `linkProgram`/`deleteProgram`.
  - Shim `WebGLRenderingContext` caches per program and clears the cache in
    `linkProgram`/`deleteProgram`.
  - Shim test: repeat lookups issue one native call; relinking invalidates.
  - Probe: 2M lookups now peak at ~132 MB instead of ~206 MB and take ~0.9 s.
- [x] L2. Image bitmaps are never released.
  - Added `native.image.free` to both bridges and `HostState::free_image`.
  - `Image.src` releases the previous bitmap; stale async loads free the
    handle they received.
  - Live handles are registered with `FinalizationRegistry`.
  - Shim test: changing `src` frees the previous native id.
- [x] L3. Native canvas surfaces are never destroyed.
  - Canvas constructors register a finalizer that calls `native.canvas.destroy`.
  - Added a deterministic `__release()` hook used by the shim test.
- [x] L4. Detached DOM nodes are retained forever.
  - Elements are registered when attached and whole subtrees are unregistered
    on detach; re-attaching registers them again.
  - `getElementsByClassName` now ignores detached elements.
  - Shim test covers attach, detach, query visibility, and re-attach.
- [x] L5. Decoded audio buffers are never released.
  - Uploaded and decoded `AudioBuffer`s register finalizers that call
    `native.audio.freeBuffer`.
  - Graph-rendered temporary buffers are disowned and freed by the voice;
    rejected voices free their owned buffers immediately.
  - Probe: all 300 decoded buffers reached the finalizer and the native free
    path.
- [x] L6. Pak decompression cache is unbounded.
  - The archive cache is capped at 64 MB with FIFO eviction.
  - Added `read_uncached` for video, audio, image, and module loads that keep
    their own bytes.
  - `read_text` reads directly into a `String` without the extra archive copy.
  - Tests cover cache eviction and the uncached read path.
- [x] L7. `AudioParam` automation history is unbounded.
  - Past events are pruned from `__currentValue`, keeping the last event as the
    interpolation anchor.
  - `__valueAt` remains pure for offline graph rendering, so rendering one
    voice cannot mutate another voice's automation.
  - The per-frame graph walk is skipped when no parameter is automating.
  - `graphParams` reuses scratch arrays and the voice-ended scan no longer
    copies the Set.
- [x] L8. Lifecycle races retain video, auth, and network resources.
  - Video opens carry a generation token; stale opens close their native
    handle and do not adopt stale audio bytes.
  - The video ready timer gives up after 10 seconds and fires `error`.
  - Auth relays cancel themselves after 10 minutes.
  - Network events use a bounded 4096-entry queue, HTTP requests a bounded
    256-entry queue, socket writes a bounded 256-entry queue, and WebSockets
    are capped at 64 open sockets.
  - `open_socket` now reports refusal back to the shim.

## Performance

- [x] P1. Cache resolved system font data.
  - System faces are copied into an `Arc<[u8]>` cache keyed by family, weight,
    and style; subsequent calls parse from memory instead of reopening and
    mmapping the font.
  - Probe: 2,000 system-font fills dropped from ~501 ms to ~327 ms.
- [x] P2. Remove text draw copies and duplicate metrics work.
  - `draw_text` borrows the 2D context instead of cloning it.
  - `FontStore::draw` resolves the face once and computes alignment width in
    the same callback.
- [x] P3. Remove repeated uniform/attribute bridge traffic.
  - Covered by L1's shim cache: repeat lookups no longer cross the bridge.
- [x] P4. Cache gamepad snapshots between state changes.
  - Native `Gamepads` tracks a revision and preserves the previous timestamp
    when axes/buttons are unchanged.
  - The shim caches both the parsed snapshot list and the gamepad array until
    the revision changes.
  - Rust test covers state comparison; shim test verifies repeat calls reuse
    the array without another `list()` call.
- [x] P5. Reduce audio graph work.
  - Covered by L7.
- [x] P6. Avoid the full pixmap copy in `drawImage(canvas)`.
  - Source and destination canvases are borrowed with disjoint mutable access.
  - Canvas data is marked premultiplied so it is not multiplied again.
  - Added an alpha regression test.
- [x] P7. Encode screenshots once.
  - `HostState` caches the encoded framebuffer PNG between `screenshotLength`
    and `copyScreenshot`, and the canvas PNG between `pngLength` and `copyPng`.
  - `toBlob` consumes those bytes directly instead of base64-encoding and
    decoding them.
  - Canvas PNGs use the faster `png` compression setting.
- [x] P8. Remove the redundant fetch body copy.
  - `makeResponse` now views the existing bytes; `arrayBuffer()` still returns
    a copy so callers cannot mutate the response body.
- [x] P9. Consume decoded image data with `into_rgba8`.
- [x] P10. Smaller hot-path allocations.
  - [x] `Watchdog` stores `&'static str` phases instead of allocating strings.
  - [x] `dispatch` no longer clones the event name.
  - [x] `performance.now()` returns the cached frame time while a frame runs.
  - [x] V8 global calls fill a fixed argument buffer instead of allocating a
    `Vec` for the common case.
  - [x] The pak text path avoids the extra `read_text` copy.

## Verification

- [x] `cargo test` for pak, canvas2d, gfx, net, audio, and host.
- [x] `cargo clippy --workspace --all-targets`.
- [x] `npm test` in `shim/` (84 tests).
- [x] Leak probes: image churn, uniform handles, DOM churn, canvas release,
      and audio buffers.
  - `Image.src` churn: ~342 MB -> ~135 MB.
  - 2M uniform lookups: ~206 MB -> ~132 MB; DOM churn: ~262 MB -> ~133 MB.
  - The canvas release test confirms `native.canvas.destroy` is called.
  - 300 decoded buffers reached both the JS finalizer and native free path.
- [x] Rebuild the release host and run Frax: 1,500 frames at 63.8 fps,
      ~10.1 ms mean / 9.6 ms p50, ~308 MB RSS.

The shim test script was also corrected to point at `test/shim.test.js`, since
`node --test test` no longer accepts a directory on the current Node release.

## Automated regression check

- `scripts/leak-check.mjs` runs light and heavy churn scenarios against the
  release host and compares native resource counts after a forced GC.
- It covers image bitmaps, canvases and their contexts, uniform locations, and
  decoded audio buffers.
- The Linux `Resource leaks` CI job runs it under Xvfb with software OpenGL.
