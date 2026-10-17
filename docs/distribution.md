# Distributing games

A shipped game is a single executable per platform: the `kaplay-native-host`
binary with your game archive and a small config appended to it. There is no
runtime to install and no extra files next to it.

## Install the CLI

```sh
npm install --global kaplay-native
kaplay-native --help
```

Or run it without installing:

```sh
npx --yes kaplay-native pack ./my-game
```

The CLI is a thin Node script. When a command needs the native host and none is
found locally, it downloads the host binary for the current platform from the
GitHub release that matches the CLI version and caches it:

- Linux: `~/.cache/kaplay-native/<version>` (or `$XDG_CACHE_HOME`)
- macOS: `~/Library/Caches/kaplay-native/<version>`
- Windows: `%LOCALAPPDATA%\kaplay-native\<version>`

Overrides: `--host <path>`, `KAPLAY_NATIVE_HOST`, and `KAPLAY_NATIVE_CACHE`.
`kaplay-native doctor` prints which host was resolved and where it came from.

Each release ships two host flavors per platform: the default QuickJS binary
(`kaplay-native-host-<platform>`) and the V8 binary
(`kaplay-native-host-<platform>-v8`). Every V8 host links against the slim
archive published by the `Slim V8` workflow, which drops i18n, WebAssembly and
Temporal and adds roughly 20 MB instead of the ~40 MB the prebuilt archive
adds. The CLI downloads the default one unless the command asks for V8 with
`--js-engine v8`, so a game built for the default engine never carries the V8
runtime.

## Pack a game

```sh
kaplay-native pack ./my-game --out dist/my-game --js-engine v8
```

`pack` bundles the game the same way `build` does (esbuild when available),
embeds the archive and config in a copy of the host, and marks the result
executable. On Windows the output gets a `.exe` suffix automatically.

Options that change what the game does when launched:

| Flag | Effect |
| --- | --- |
| `--js-engine quickjs\|v8` | Engine and host flavor for this binary |
| `--title <name>` | Window title |
| `--window <WxH>` | Initial window size |
| `--allow-network` | Enable HTTP and WebSocket requests |
| `--url <url>` | Page URL for `window.location` and relative requests |
| `--game-id <id>` | Stable id for saves and content updates |
| `--content-version <v>` | Version of the packaged content (semver) |
| `--update-url <url>` | Manifest URL checked on launch |
| `--update-key <key>` | Ed25519 public key (base64 or a `.pub` file) |

`--js-engine v8` also selects the V8 host flavor when the CLI downloads one.
The result contains only that engine, so a QuickJS-packed game cannot switch to
V8 at runtime; pack with `--js-engine v8` to ship V8.

Everything else (`--host`, `--out`, `--entry`, `--web-dist`, `--pak`) controls
packaging itself. Pack from an existing archive with `--pak game.pak`.

At runtime the embedded defaults sit between the command line and the built-in
defaults, so all of these still work on a packed game:

```sh
./my-game --window 1280x720 --stats
./my-game --pak other-game.pak
./my-game --screenshot frame.png --frames 120
```

If the game ships with a backend, pack with `--allow-network` and `--url` so
double-clicking the game connects without extra flags.

## The packed format

The output is the host executable followed by three records:

```
[host executable][game .pak][config JSON][trailer: 24 bytes]
```

The trailer is little-endian:

| Offset | Size | Value |
| --- | --- | --- |
| 0 | 8 | `KNATIVEP` |
| 8 | 4 | format version (`1`) |
| 12 | 8 | pak length in bytes |
| 20 | 4 | config length in bytes |

The config is a JSON object; every key is optional: `jsEngine`, `title`,
`window`, `network`, `url`, `gameId`, `contentVersion`, and `update`
(`url` plus `publicKey` or `publicKeys`). Explicit command-line flags win over the config,
which wins over the host defaults. A `--pak` or `--dir` argument also
overrides both the embedded and the installed archive, which is useful for
testing a new build against an existing binary.

`gameId` moves saves to `data_dir()/kaplay-native/<gameId>`. When `update` is
present, the host also checks that manifest on launch and installs newer content
for the next start. See [updates.md](updates.md) for the key, manifest, and
publishing workflow.

The host only reads the trailer from its own executable, so a plain
`kaplay-native-host` binary is unaffected.

On macOS, appending to a Mach-O binary invalidates its ad-hoc signature, so
`pack` runs `codesign --force --sign -` on the output. That is enough to run
locally and in CI; shipping outside your team still needs a Developer ID
signature and notarization.

## Content updates

A packed game can update its `.pak` on launch without replacing the executable.
Pack with `--game-id`, `--content-version`, `--update-url`, and `--update-key`,
publish a signed manifest, and players pick up new content on their next launch.
Updates are skipped for Steam launches, headless runs, and explicit `--pak` or
`--dir` runs. `--reset-content` returns a player to the content embedded in the
executable, and `--no-update` disables the check. Games can read
`kaplayNative.updates.status()` and call `kaplayNative.updates.apply()` to
restart into a downloaded update. The full workflow is in
[updates.md](updates.md).

## Game CI

This workflow builds a single-file game for Linux, Windows, and Apple Silicon
macOS whenever the game is tagged. Add Intel macOS by adding `macos-15-intel`
to the matrix.

```yaml
name: Release

on:
  push:
    tags: ["v*"]

jobs:
  package:
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-latest
            artifact: my-game-linux-x64
          - os: windows-latest
            artifact: my-game-windows-x64.exe
          - os: macos-latest
            artifact: my-game-macos-arm64
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 22
      - name: Package
        run: npx --yes kaplay-native pack . --out "${{ matrix.artifact }}" --js-engine v8
      - uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.artifact }}
          path: ${{ matrix.artifact }}
```

Swap `actions/upload-artifact` for a GitHub release step if you want tagged
builds published, and add `--allow-network --url https://your-server` if the
game talks to a backend.

### Cross-packaging

`--host` accepts a host binary for another platform, so one machine can produce
every artifact. Download the hosts from the
[releases page](https://github.com/beemdvp/kaplay-native/releases) and name the
output yourself:

```sh
kaplay-native pack ./my-game --host hosts/kaplay-native-host-windows-x64.exe \
  --out dist/my-game.exe --js-engine v8
```

## Releasing the toolchain

Merging to `main` releases the next version. The `Publish` workflow uses the
version already in `cli/package.json` when a pull request set one, otherwise it
bumps the prerelease or patch number, updates the Cargo workspace version and
`Cargo.lock` with it, pushes the `v<version>` tag, and dispatches the release
workflow. That workflow builds the QuickJS and V8 host for Linux x64 and arm64,
macOS x64 and arm64, and Windows x64, runs a packed-game smoke test (V8 flavor)
on each platform, and attaches the binaries to the GitHub release:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

Tags can still be pushed by hand this way; the `Publish` workflow only
automates the bump and tag. It needs an `NPM_TOKEN` repository secret (an npm
automation token with publish rights for `kaplay-native`), which the release
workflow uses to publish `cli/`. Without the secret the hosts are still built
and released, and npm publishing is skipped. The npm version always matches the
tag because the release workflow rewrites `cli/package.json` from the tag
before publishing.

Every V8 host links against the slim archive published by the `Slim V8`
workflow when a `slim-v8-<version>` release exists, and falls back to the
prebuilt V8 archive otherwise. The workflow builds the archive natively on each
platform, and cross-builds the macOS x64 archive on the arm64 runner. Run
`Slim V8` once after upgrading the `v8` crate; it smoke-tests the archive
against QuickJS before publishing it.
