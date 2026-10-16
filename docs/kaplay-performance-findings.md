# KAPLAY performance findings

Basis for optimising the KAPLAY JS runtime so KAPLAY Native games perform
better. Everything here is measured on Linux x64 / AMD Renoir against the
KAPLAY build at git `45dad2ca` (`kaplay.js`, 301 037 bytes, sha256
`5971ba8abb549667`).

The measurement basis is the KAPLAY scenario suite added alongside this file:

```sh
node scripts/bench-kaplay.mjs baseline --engine both --no-build
node scripts/bench-kaplay.mjs compare  --engine quickjs --no-build
node scripts/bench-kaplay.mjs run      --engine quickjs --scenario draw
```

`benchmarks/kaplay/main.js` builds a controlled scene per scenario and the
host's `--bench-json` reports the frame and phase percentiles. Deterministic
`BENCH objects` / `BENCH checksum` lines guard functional parity: they are
stable across runs and identical between engines. Baselines live in
`benchmarks/baselines/kaplay-linux-x64-both.json`. Each scenario is 300 frames
(10 for `collide`) at 960x540, headless, uncapped, no audio, and runs twice
with the lower JS p50 kept.

`js p50` below is the KAPLAY callback p50 reported by the host, in ms. The
`micro*` scenarios call one public API 400 times per frame from a single draw
handler with no ECS around it, so `loop_ms / 400` is the per-call cost.

| Scenario | quickjs js p50 | quickjs per call | v8 js p50 | v8 per call |
| --- | --- | --- | --- | --- |
| `idle` | 0.17 | — | 0.16 | — |
| `draw` (400 sprites) | 19.69 | 49.2 µs | 2.91 | 7.3 µs |
| `move` (400 sprites + updates) | 22.27 | 55.7 µs | 4.04 | 10.1 µs |
| `text` (400 text objects, 17 glyphs) | 157.77 | 0.39 ms | 8.51 | 21.3 µs |
| `collide` (200 overlapping areas) | 487.03 | 2.44 ms | 14.26 | 71.3 µs |
| `churn` (50 add + 50 destroy / 2 frames) | 3.44 | — | 1.28 | — |
| `events` (400 objects × 3 `onUpdate`) | 4.12 | — | 0.78 | — |
| `microquad` (`drawUVQuad`) | 8.00 | 16.9 µs | 0.45 | 0.71 µs |
| `microsprite` (`drawSprite`) | 11.40 | 25.9 µs | 0.75 | 1.41 µs |
| `micromatrix` (`pushTransform`/`pushTranslate`/`popTransform`) | 0.65 | 1.15 µs | 0.15 | 0.04 µs |

QuickJS is the default native engine and it is 6-34x behind V8 on every hot
path, so interpreter op count is what matters most for KAPLAY Native. V8
numbers are recorded so upstream-friendly changes can be judged on both.

Everything below is PR-sized: one problem, one solution, one reason. The
numbers were gathered with the baseline command above, and the temporary
instrumentation used to attribute them is not part of this tree.

## 1. Sprite drawing costs 25 µs per sprite on QuickJS

### Problem

400 static sprites spend 19.69 ms of a 20.20 ms frame in JS. The leaf costs
are isolated by the micro scenarios: `drawSprite` is 25.9 µs per call and
`drawUVQuad` 16.9 µs per call on QuickJS (1.41 µs and 0.71 µs on V8). The
path `sprite` component draw → `drawSprite` → `drawTexture` → `drawUVQuad` →
`drawRaw` → `BatchRenderer.push` performs, per sprite:

- two `Object.assign` copies of the option object (`sprite.ts`,
  `drawTexture.ts`) plus `getRenderProps` and a fallback `new Quad(0, 0, 1, 1)`;
- four attribute arrays plus an index array in `drawUVQuad`;
- a fresh `vv` array in `drawRaw`, a `transformPointV` call per vertex;
- a second copy into `BatchRenderer.vqueue`/`iqueue` with one `Array.push` per
  element, plus the generic texture/shader/uniform/blend/fixed comparisons.

Batching itself is fine: a 400-sprite scene issues 2 flushes per frame, so
this is interpreter cost, not GL traffic or GC. Two measured prototypes
(reusing module-level scratch arrays for the attribute arrays, and inlining
the matrix coefficients) moved the 400-sprite frame by only ~1% and ~5%
total on QuickJS and nothing measurable on V8. What is left is the per-element
`Array.push` copy in `BatchRenderer.push` plus the generic per-draw checks, so
the quad fast path is the required change rather than a local allocation
cleanup.

### Solution

- Inline the matrix coefficients in `drawRaw` (`const { a, b, c, d, e, f }`)
  and write the transformed vertices straight into the renderer queue instead
  of allocating `vv` and copying it again.
- Add a quad fast path to `BatchRenderer` that takes the four corners, the uv
  rect, the color and opacity and writes the 32 interleaved floats and the 6
  indices in one pass, so `drawUVQuad` no longer builds generic attribute
  objects.
- Delete the intermediate `Object.assign` in the sprite component draw and in
  `drawTexture`, and use a module-level constant for the default quad.

### Why

Sprites are the most common drawable in 2D games, and at 400 sprites the JS
phase is 97% of the frame on the default native engine. Cutting the per-sprite
op count directly multiplies the sprite budget of every native game, and the
same path feeds every other quad-based drawable (tiles, rectangles, particles).

## 2. Text redraws 17 glyph quads and regroups them every frame

### Problem

400 static text objects cost 157.77 ms per frame on QuickJS (8.51 ms on V8),
about 0.39 ms per object, with no text changing. Two costs are visible:

- `drawFormattedText` rebuilds a `Map<Texture, FormattedChar[]>` and an
  `Array.from(...).flat()` every draw, even though the formatted text only
  changes when `formatText` produces a new `FormattedText`;
- each glyph is a full `drawUVQuad` call, so the per-glyph cost is the 16.9 µs
  from finding 1: roughly 17 glyphs × 16.9 µs ≈ 0.29 ms of the 0.39 ms.

### Solution

- Store the texture-grouped glyph order on the `FormattedText` when it is
  built (or invalidate it from `transformFormattedText`) and iterate that in
  `drawFormattedText` instead of rebuilding the map and arrays per frame.
- Feed the glyphs through the quad fast path from finding 1.
- Do the low-risk per-object cleanup at the same time: `anchorPt(...).add(1, 1)
  .scale(width, height).scale(-0.5)` in `drawFormattedText` allocates three
  `Vec2`s per text per frame and can be four multiplications.

### Why

A HUD with dozens of labels or a dialogue box already costs milliseconds per
frame on QuickJS. Static text is the common case: it should pay layout once at
format time and only the quads at draw time.

## 3. The ECS traversal allocates for every object every frame

### Problem

`GameObjRaw.update` and `GameObjRaw.fixedUpdate` call `this.children.slice()`
for every node on every step, and `drawTree` builds a fresh `objects` array
and sorts it on every node on every frame, although layer and `z` rarely
change. For 400 sprites:

- 400 `update()` calls allocate 400 arrays; 400 `fixedUpdate()` calls the
  same. A flat scene's V8 draw frame is 2.91 ms while its 400 leaf
  `drawSprite` calls only cost 0.56 ms, so ~2.3 ms is traversal, dispatch and
  sorting; on QuickJS the draw phase is ~9.3 ms above the leaf calls.
- `drawTree`'s `objects.sort` comparator also reads `o1._drawLayerIndex`,
  `o1.z`, `o2._drawLayerIndex`, `o2.z` for every comparison.

### Solution

- Iterate children by index with a cached length; destroyed children are
  already skipped through `exists()`/`paused` guards, and additions during
  iteration should be deferred to the next frame instead of copying the array.
- Keep the collected draw list per tree in a reusable buffer instead of
  allocating one array per node, and only re-sort when a child is added, a
  layer changes or a `z` changes; store a per-object sort key (`layerIndex`,
  `z`) so the comparator does no lookups.

### Why

This cost scales with object count on every frame and hits both engines, but
it is proportionally worse on V8 where the leaf draw is cheap. Removing it is
also the cheapest way to reduce GC pressure, which shows up as native p99
spikes.

## 4. The collision pass runs twice per frame and allocates per pair

### Problem

The collision system is registered for both `AfterFixedUpdate` and
`AfterUpdate` (`kaplay.ts`), so broad + narrow phase runs after every fixed
step *and* after every rendered frame. 200 overlapping `area()` circles cost
487.03 ms per frame on QuickJS and 14.26 ms on V8. Per intersecting pair the
narrow phase creates two `Collision` objects and a negated normal `Vec2`
(`Collision.reverse`), and `SweepAndPruneHorizontal.iterPairs` allocates a new
`Set` per pass and calls `isValidCollisionObject` per edge and per candidate,
which does `obj.has("body")` and an `isPaused` ancestor walk.

### Solution

Split into two independent changes:

- Run the collision pass once per fixed step and skip it on the frame pass
  when no area moved; keep the callback semantics of `collideUpdate` intact.
- Reuse the broad-phase `Set`s, make `Collision.reverse` lazy (only allocate
  the reversed normal when someone reads it) and cache the effective paused
  state instead of walking the parent chain per candidate.

### Why

The cost is O(pairs) with heavy per-pair JS, and it is the single worst
scenario in the suite: at 200 overlapping circles QuickJS drops to 2 fps and
V8 to 33 fps. Native physics games (Frax is collision-heavy) pay this on the
main thread every frame.

## 5. Event dispatch pays a wrapper, a spread and a pause walk per handler

### Problem

`KEvent.trigger` invokes a per-registration wrapper that spreads `...args` and
evaluates `ev.paused || ev.controller && isPaused(ev.controller)` before each
callback. `isPaused` recurses through the parent chain every time. The
`events` scenario (400 objects × 3 `onUpdate` listeners) costs 4.12 ms per
frame on QuickJS and 0.78 ms on V8; even an empty 400-object scene pays for
every object's update/draw trigger.

### Solution

- Maintain an effective paused flag on each object when the pause state
  changes (the `_updatePausedState` machinery already recurses the tree) and
  read it in the wrapper instead of walking parents.
- Give `KEvent` a zero-argument fast path for update/draw events so the
  wrapper and spread are not created, and set the controller reference at
  registration instead of wrapping in a new closure.

### Why

Update and draw events fire for every object every frame; for scenes with a
thousand objects this is milliseconds of pure bookkeeping on QuickJS and a
measurable share on V8, before any user callback runs.

## 6. Loading costs: `add()` is 0.38 ms and a text object 1.8 ms on QuickJS

### Problem

The scenario setup times are high: adding 400 sprites takes ~150 ms QuickJS
(~50 ms V8), adding 400 text objects with the default font ~725 ms QuickJS
(~160 ms V8), and 400 objects with three listeners each ~64 ms (~21 ms V8).
Text is dominated by the first-use font atlas work in `formatText` (a
`canvas2d` `measureText` + `fillText` + `getImageData` per new glyph on the
native side, where a long `fillText` measures at 858 µs). `add()` runs the
component hooks, the proxied property setup and the event registrations per
object.

### Solution

- Cache formatted text by `text` + the option tuple when no per-character
  transform/style is set, so repeat labels reuse the `FormattedText` and the
  glyph atlas entries instead of re-walking the formatter.
- Avoid re-running full component initialization for identical component
  tuples where the component already supports copy (for example static
  `text(label)` HUDs).
- Profile `add()` itself before changing it; the setup numbers include the
  per-component closures and proxies and need their own attribution.

### Why

Native games load levels and scenes at runtime, and the ~725 ms QuickJS figure
for 400 labels is a visible hitch, not a theoretical one. It is listed after
the frame-time findings because it runs once per load rather than per frame.

## 7. Minor per-frame allocations in the frame pipeline

### Problem

`frameRendering.frameStart` allocates a `Quad` per frame to draw the default
background grid when no background color is set, `drawFrame` allocates two
`Vec2`s and calls `rand()` and `Vec2.fromAngle` for the camera shake even when
`cam.shake` is 0, and `resetInput` allocates a `Vec2` for the mouse delta plus
a `Vec2` per stick per frame. Individually these are below 0.1 ms, but they
run on every frame of every game.

### Solution

Share a module-level `Quad` for the background grid, guard the camera shake
math behind `cam.shake !== 0`, and reuse the mouse-delta `Vec2`.

### Why

They are trivial, safe changes that remove work from every frame of every
game; land them together as one small cleanup rather than mixing them into the
larger draw and ECS changes.

## Verification

For each change, compare back-to-back against the baseline recorded on the
same machine:

```sh
node scripts/bench-kaplay.mjs baseline --engine both --no-build
# apply the change, rebuild KAPLAY with npm run build
node scripts/bench-kaplay.mjs compare  --engine both --no-build
```

`compare` fails on any deterministic `objects`/`checksum` mismatch (functional
regression) and on a >10% regression in the JS phase p50 or in a micro
`loop_ms`, warns above 5%, and reports both engines; `fps` and the frame
percentiles are reported but never gate. Absolute numbers drift between
sessions on this laptop, so always record the baseline with the build being
changed and keep the KAPLAY bundle hash
(`benchmarks/baselines/kaplay-linux-x64-both.json`) with the result.
