# Examples

- `hello` - a KAPLAY game: rectangles, a circle, and text. Requires a KAPLAY
  bundle passed with `--preload`.
- `smoke` - raw WebGL without KAPLAY: clears the screen, uploads a textured
  quad, and captures a screenshot. Used as the CI smoke test.
- `texupload` - focused texture-upload acceptance test: `texImage2D` with raw
  typed arrays, `texSubImage2D` with the width/height overload, `ImageData`,
  and offset sub-updates, each rendered to a quadrant and verified with
  `readPixels`. Runs identically on both engines.
- `imageload` - async image-decode acceptance test: loads a PNG data URL
  through `Image.src`, verifies dimensions and decoded pixels via a GL
  texture readback, and checks that a src switch after load still completes.
- `storage` - debounced localStorage acceptance test: writes 201 items in one
  frame; run it twice with the same `--save-dir` to verify the pre-exit flush
  persisted everything.
- `modules` - a multi-chunk ES module build: the entry imports a sibling chunk,
  loads another chunk with `import()`, checks that both share one chunk
  instance, and reads the canvas and title from `index.html`. Used as the CI
  module-resolution test.
- `video` - decodes an H.264 MP4 with `document.createElement("video")`, uploads
  the frame with `gl.texImage2D`, and reads the pixel back. Used as the CI
  video-playback test.
- `steam` - KAPLAY demo for Steamworks: player/stat/achievement and cloud
  status, with Space/A/S/D to write the cloud file, unlock
  `ACH_WIN_ONE_GAME`, add a `NumWins` stat, and delete the cloud file. Requires
  a KAPLAY bundle via `--preload`; easiest to launch with
  `scripts/steam-overlay-test.sh --demo` (app id 480).

```sh
cargo run -p kaplay-native-host -- --script examples/smoke/main.js --dir examples/smoke --frames 5 --screenshot smoke.png
cargo run -p kaplay-native-host -- --dir examples/modules --frames 1
cargo run -p kaplay-native-host -- --dir examples/video --frames 40
cargo run -p kaplay-native-host -- --script examples/hello/main.js --preload path/to/kaplay.js --dir examples/hello
```
