# Compatibility

KAPLAY Native implements the browser surface KAPLAY uses, not the whole web
platform. This page lists what works today and what happens when something is
missing.

## JavaScript

- QuickJS via `rquickjs` with ES2020, or V8 via `rusty_v8` with full modern
  JavaScript, selected at build time and runtime. The engine matrix and build
  flags are in [engines.md](engines.md).
- The game entry is evaluated as an ES module, matching `<script
  type="module">` in a browser: top-level declarations stay module-scoped and
  cannot clobber globals exported by KAPLAY or other libraries. `--preload`
  scripts stay classic scripts, like `<script src>` on a page; a preload that
  is only valid as a module is evaluated as one and its named exports become
  globals.
- Static `import` statements and dynamic `import()` calls resolve from the
  game files: `./x.js` and `../x.js` are relative to the importing module,
  `/x.js` is relative to the asset root, and bare specifiers must be bundled.
  Both engines share the same resolution rules, and a missing module reports
  the specifier it could not load.
- `Promise`, `async`/`await` with an explicit job queue pump on both engines.
- `TextEncoder` and `TextDecoder` (UTF-8) are shimmed. QuickJS does not provide
  them, and the shim keeps them unconditional so V8 behavior is identical.
- `Promise.withResolvers` is polyfilled on both engines.
- `URL` and `URLSearchParams` are implemented, including base resolution and
  form encoding, because SDKs parse their endpoint URLs at import time.
- `crypto.getRandomValues` and `crypto.randomUUID` are backed by `Math.random`,
  which is fine for gameplay identifiers but not for secrets.
- `matchMedia` returns a non-matching media query list, so pointer-capability
  checks report a desktop.
- `Intl` is not available on the default QuickJS build. KAPLAY falls back to a
  code-point splitter in `runes.ts`, so text still works, but locale-aware
  segmentation does not. The V8 backend provides `Intl`.

## DOM and events

- `document`, `document.head`, `document.body`, `document.documentElement`
- `index.html` is parsed into `document` before the game runs: elements,
  attributes, `class`, `id`, `title`, `classList`, and
  `querySelector`/`querySelectorAll` for `tag`, `#id`, `.class`, and `[attr]`
  selectors. Inline scripts and CSS are not executed.
- `createElement`, `createTextNode`, `getElementById`, `getElementsByTagName`,
  `getElementsByClassName`, `querySelector`, `appendChild`, `prepend`,
  `removeChild`, `insertBefore`, `setAttribute` for `div`, `input`, `textarea`,
  and `form` elements
- `Event`, `MouseEvent`, `KeyboardEvent`, `WheelEvent`, and `CustomEvent`
- canvas creation, `appendChild`, `getBoundingClientRect`
- `style` as a flat CSS property bag: direct properties, `cssText`,
  `setProperty`/`getPropertyValue`/`removeProperty`, and assigning a CSS string
  to `style` parses it like `cssText`. There is no cascade, layout, or DOM
  rendering; only `cursor` and canvas `width`/`height` affect the native side.
- `document.activeElement`, `focus()`, `blur()`
- `addEventListener`/`removeEventListener` on canvas, document, and window
- `dispatchEvent`
- `document.visibilityState` and `visibilitychange`
- `ResizeObserver`
- `MutationObserver` (records are not observed; the callback API is a stub used
  by bundler polyfills)
- `canvas.requestPointerLock()` and `canvas.requestFullscreen()`, with
  `document.pointerLockElement`/`fullscreenElement`, `pointerlockchange` and
  `fullscreenchange` events, `exitPointerLock`/`exitFullscreen`, and the
  `webkit` aliases
- `quit()` and `window.close()` stop the host; removing the main canvas also
  exits, which is how KAPLAY's `quit()` behaves natively
- `requestAnimationFrame`, `setTimeout`, `setInterval` and cancellations

Pointer events carry `clientX/clientY`, `offsetX/offsetY` scaled through the
drawn canvas rectangle, `movementX/movementY`, and a `buttons` bitmask.
Keyboard events carry `key`, `code`, `repeat`, modifier flags, and
`getModifierState`.

`navigator.getGamepads()` exposes standard-mapped gamepads with `id`, `index`,
`mapping`, `timestamp`, four stick axes, and 17 buttons (digital and analog
values). `window` receives `gamepadconnected` and `gamepaddisconnected` events,
and KAPLAY's gamepad bindings and sticks work through the usual polling. The
list is refreshed from `gilrs` every frame; rumble/haptic output is not
exposed yet.

While an `input` or `textarea` is focused, keys go to it like they do in a
browser: printable keys and Backspace edit `value` and fire `input` (Enter adds
a line break to a `textarea`), `maxLength` is enforced, and the canvas does not
receive those keys. A click inside a positioned field focuses it, which is how
a field that a game draws with canvas graphics still receives focus; a click
anywhere else releases focus so the game gets its keys back. DOM elements are
not rendered, so games draw their own fields and keep the DOM field as an
invisible input over them.

Setting `cursor` on a `style` (`style.cursor`, or `cursor: ...` inside
`cssText`) changes the real OS cursor. `none` hides it, which is what
`fakeMouse` games rely on, and the standard keywords map to platform cursor
icons.

Window resizes update `innerWidth`/`innerHeight`, `screen.width`/`height`/`dpr`,
the canvas box reported to `ResizeObserver`, and dispatch a `resize` event on
`window`, so KAPLAY's `onResize` and any `ResizeObserver`-based layout run
exactly as they do in a browser.

Wheel events carry `deltaX`, `deltaY`, and `deltaMode`.

`window.open(url, ...)` opens the URL in the system default browser. There is
no renderer for web pages inside the game process, so it returns a window-like
stub whose `closed` flag flips to `true` once the flow finishes; code that
waits for a `message` event from the opened page still works through a
loopback relay. That is what makes popup-based OAuth flows (for example
Colyseus/Supabase `signInWithProvider`) work: the runtime serves a small
sign-in page on `127.0.0.1`, the system browser opens the provider login, and
the provider's callback page posts the `{ user, token }` payload back to the
game. `window.open` returns `null` when a flow is already in flight, when the
browser cannot be started, or for non-`http(s)` URLs.

## Canvas 2D

`clearRect`, `fillRect` and `strokeRect` (no-ops used by KAPLAY's background
helpers), `fillText`, `strokeText`, `measureText` (width plus
`actualBoundingBoxAscent`/`Descent`), `drawImage` in 3, 5, and 9 argument
forms, `getImageData`, `putImageData`, style setters, and `toDataURL`/`toBlob`
backed by the native surfaces. Path helpers exist as no-ops so accidental use
does not throw.

## WebGL 1

The full surface KAPLAY uses: textures, buffers, shaders and programs,
uniforms, vertex attributes, framebuffer and renderbuffer objects, state
toggles, `readPixels`, and `getExtension("WEBGL_lose_context")`.

The context is desktop OpenGL. Shaders are translated from GLSL ES 1.00 for
both compatibility profiles (`#version 120`) and core profiles (`#version 150
core` with `in`/`out` and a declared fragment output); the profile is detected
from the context, and core profiles get a default vertex array object because
one is required there. macOS always yields a core profile through glutin, so
that path is exercised by CI on every run.

`getExtension` returns `null` for every other extension, `getError` always
returns `NO_ERROR`, and `isContextLost` returns `false`.

## Assets and resources

- `fetch` for data URLs, blob URLs, bundled files, and HTTP(S) URLs.
- HTTP(S) requests require `--allow-network`; without it they reject with a
  `TypeError` that names the flag.
- `--url` sets the page URL (`window.location`). Relative requests first check
  the asset root and then resolve against that URL, so bundled sprites keep
  loading from disk while API calls go to the game server.
- `Headers`, `Request`-style `fetch(url, init)` options (`method`, `headers`,
  `body` as string/`URLSearchParams`/`Blob`/`ArrayBuffer`/typed array), response
  `json()`, `text()`, `arrayBuffer()`, `blob()`, `ok`, `status`, `statusText`,
  and `AbortController` cancellation are supported. Response bodies are read on
  the network thread and copied into the JS array buffer on the frame that
  delivers the response.
- `WebSocket` implements the browser surface: `readyState` constants,
  `binaryType` (`arraybuffer` or `blob`), `send` for strings and binary data,
  `close(code, reason)`, and `onopen`/`onmessage`/`onerror`/`onclose` plus
  `addEventListener`. Plain `ws://` and TLS `wss://` connections are both
  supported.
- `Image` with data URLs, blob URLs, bundled files, and HTTP(S) URLs when
  `--allow-network` is set. PNG, JPEG, WebP, GIF (first frame), and BMP are
  decoded, and `naturalWidth`/`naturalHeight` are populated.
- `FontFace` from an `ArrayBuffer`/typed array, a data URL, a bundled file, or
  an HTTP(S) URL when `--allow-network` is set.
- `download`, `downloadText`, `downloadJSON`, and `downloadBlob` write to the
  platform downloads directory, falling back to the save directory. Existing
  files are not overwritten; a counter is appended to the suggested name.
- `localStorage` is persisted as JSON in the platform data directory
  (`--save-dir` overrides it) and written atomically.
- `AudioContext`, `AudioBuffer`, `AudioBufferSourceNode`, `GainNode`,
  `StereoPannerNode`, `OscillatorNode`, `BiquadFilterNode`, `DelayNode`,
  `WaveShaperNode`, `DynamicsCompressorNode`, `ConvolverNode`,
  `ChannelSplitterNode`, `ChannelMergerNode`, `IIRFilterNode`, `PannerNode`,
  `ConstantSourceNode`, `AnalyserNode`, `PeriodicWave`,
  `MediaElementAudioSourceNode`, and `AudioParam`. `start(when, offset,
  duration)`, `stop`, `loop`, `loopStart`/`loopEnd`, `playbackRate`, `detune`,
  `pan`, `gain`, `delayTime`, `offset`, source positions,
  `setValueAtTime`/`linearRampToValueAtTime`/`exponentialRampToValueAtTime`,
  `connect(node, output, input)`/`disconnect`, `onended`, and
  `getChannelData`/`copyFromChannel`/`copyToChannel` work.
- Sources and effect chains that need sample processing render to PCM when
  playback starts. `OscillatorNode` supports sine, square, sawtooth, triangle,
  and custom `PeriodicWave`; `BiquadFilterNode` supports lowpass, highpass,
  bandpass, notch, allpass, peaking, lowshelf, and highshelf. Delay,
  waveshaping, dynamics compression, convolution, IIR filtering, panning,
  channel split/merge, and constant sources are evaluated over the rendered
  note with automation. Rendered notes are limited to 30 seconds, convolution
  impulse responses to 5 seconds, and feedback loops through a `DelayNode` are
  rejected. Parameter changes made after a rendered voice starts are not
  applied to that voice; construct and connect the graph before `start()`.
- `AnalyserNode` reads the mixed master output through the native audio engine
  (`fftSize` from 32 to 32768, time-domain and frequency-domain byte/float
  data, smoothing, and dB range). The analyser is a master tap, not a tap at
  its position in the graph.
- `HTMLAudioElement` (`new Audio(url)`) plays music through the same mixer:
  `play`, `pause`, `currentTime`, `duration`, `volume`, `playbackRate`, `loop`,
  and `onended`.
- `HTMLVideoElement` (`document.createElement("video")`) plays H.264 MP4 files
  bundled with the game (or served over HTTP(S) with `--allow-network`):
  `play`, `pause`, `load`, `currentTime`, `duration`, `loop`, `muted`,
  `volume`, `playbackRate`, `videoWidth`, `videoHeight`, `currentSrc`,
  `readyState`, `networkState`, `error`, `canPlayType("video/mp4")`, and the
  `loadedmetadata`, `loadeddata`, `canplay`, `playing`, `seeking`, `seeked`,
  `timeupdate`, `ended`, and `error` events. A worker thread decodes ahead of
  the playhead; `readyState` reports `HAVE_CURRENT_DATA` once the frame at
  `currentTime` is decoded, so a paused seek resolves through `seeked` when its
  frame arrives. Frames are drawn with `gl.texImage2D(target, level,
  internalformat, format, type, video)`, which is the path KAPLAY's `video()`
  component uses. An audio track plays through the same mixer as
  `HTMLAudioElement`; a silent file plays without one. WebM, VP8/VP9, and AV1
  are not supported, and `canPlayType` and
  `navigator.mediaCapabilities.decodingInfo` report MP4 support only.
- Decoding supports WAV, MP3, FLAC, Ogg Vorbis, Opus, and the AAC track of an
  MP4, from bundled files, data URLs, and HTTP(S) URLs.
- `AudioWorklet`, `ScriptProcessorNode`, `AudioContext.createMediaStreamSource`,
  `createMediaStreamDestination`, and `record()`/`MediaRecorder` are not
  implemented. Without an audio device the engine runs silently and playback
  still advances in real time.
- Touch events are not delivered from native input yet, so `isTouchscreen()`
  reports false.

## Platforms

The GL path targets desktop OpenGL everywhere: compatibility contexts on
Windows and Linux, and the core profile on macOS. On Linux, X11 and Wayland are
supported through winit and glutin; `--x11` forces the X11 backend (XWayland on
Wayland sessions) and `--no-vsync` uncaps the frame rate, which some Wayland
compositors otherwise ignore. The CI matrix builds, tests, and renders on
Linux, macOS, and Windows.

## Verification story

`cargo test` covers the native helpers, `node --test shim/test/*.test.js`
covers the browser semantics against a stub bridge, and the runtime can render
a fixed number of frames and capture a screenshot. CI builds the V8 backend in
addition to the default QuickJS build, runs the `examples/bridge` conformance
script on both engines, compares the `examples/smoke` frames byte for byte, and
packs `examples/bridge` into a single-file executable and runs it. There is no browser or Chromium dependency anywhere in the tree by
design: Steam overlay support requires a host that owns `SwapBuffers`, which
webview shells do not allow.
