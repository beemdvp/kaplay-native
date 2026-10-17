# Content updates

A packed game can check for new game content on launch, download it in the
background, and use it the next time the game starts. The executable stays the
same: only the `.pak` (the game's JavaScript and assets) is replaced, so updates
are small and do not touch code signing, the install directory, or the player's
saves.

Runtime (host) updates are not covered here; content updates cover everything
that changes between releases unless the game starts using a new host API.

## How it works

At boot the host resolves game content in this order:

1. `--pak <path>` or `--dir <path>` when the player passes one
2. an installed update in the user data directory
3. the `.pak` embedded in the executable

An installed update is only used when its `contentVersion` is newer than the
version embedded in the executable. If the player downloads a new executable
whose embedded content is as new or newer, the installed update is ignored,
which makes re-downloading the game reset it to the packaged version.

The update check runs on a background thread after the game has loaded. It never
blocks startup, and a failed check leaves the current content running. Checks
happen at most once every six hours per game. Updates do not require
`--allow-network`; that flag only controls the game's own `fetch` and
`WebSocket` access.

Checks are skipped when:

- the game was packed without update settings
- the player passes `--pak`, `--dir`, `--no-update`, or `--headless`
- `KAPLAY_NATIVE_NO_UPDATE` is set
- the game is launched with `--appid`, or Steam set `SteamAppId` (Steam manages
  those installs)

A downloaded update is written to a temporary file, hashed, and only then moved
into place. The next launch picks it up. The previous `.pak` is kept so manual
rollback is possible. `--reset-content` deletes installed content and runs the
embedded one for that launch; add `--no-update` to keep it there.

## Create a signing key

Updates are signed so a compromised CDN cannot ship new game code. Generate the
key pair once:

```sh
kaplay-native keygen --out update-key.pem
```

This writes `update-key.pem` (the private key) and `update-key.pem.pub` (the
32-byte Ed25519 public key in base64). Keep the private key out of the
repository and in your CI secret store. The public key is embedded in the game;
it is safe to commit.

## Pack an updatable game

```sh
kaplay-native pack ./my-game --out dist/my-game-linux-x64 \
  --js-engine v8 \
  --game-id my-game \
  --content-version 1.4.2 \
  --update-url https://cdn.example.com/my-game/stable/manifest.json \
  --update-key update-key.pem.pub
```

`--game-id` must stay the same for every release, across platforms. It also
names the save directory (`data_dir()/kaplay-native/<game-id>`), so changing it
starts saves and installed content from scratch. `--content-version` must be
semver and must increase for every release. `--update-url` is the manifest to
check, and `--update-key` accepts either the `.pub` file or the base64 key
itself.

`--game-id` defaults to a slug of `--title` when a title is set. For updates,
pass it explicitly; the CLI requires a game id and a content version when
`--update-url` is used.

## Publish an update

1. Upload the new `.pak` to an immutable URL that includes its version.
2. Write and sign a manifest that points at it.
3. Upload `manifest.json` and `manifest.json.sig` next to each other.

```sh
kaplay-native manifest dist/game-1.4.2.pak \
  --version 1.4.2 \
  --url https://cdn.example.com/my-game/1.4.2/game.pak \
  --game-id my-game \
  --min-host 0.1.0-alpha.7 \
  --key update-key.pem \
  --out dist/manifest
```

The command writes `manifest.json` and `manifest.json.sig`. `--min-host` is
optional; set it when the content needs a newer runtime. The host refuses
content whose `minHost` is newer than itself and keeps running the current
content.

A published layout looks like this:

```
/my-game/stable/manifest.json
/my-game/stable/manifest.json.sig
/my-game/1.4.2/game.pak
```

The manifest is a small JSON file:

```json
{
  "format": 1,
  "gameId": "my-game",
  "content": {
    "version": "1.4.2",
    "minHost": "0.1.0-alpha.7",
    "url": "https://cdn.example.com/my-game/1.4.2/game.pak",
    "sha256": "…",
    "size": 8123456
  }
}
```

The signature covers the exact bytes of `manifest.json`. The host fetches
`<manifest-url>.sig`; when the manifest URL has a query string, `.sig` is
inserted before the `?`.

## Verification and failure handling

The host verifies, in order:

1. the manifest signature against the public key embedded at pack time
2. the manifest format and `gameId`
3. that the content version is greater than the current one
4. that `minHost` is not newer than the running host
5. the downloaded size and SHA-256

A failure at any step is logged and the current content keeps running. Updates
cannot downgrade content and cannot be swapped in while the game is running.

Keep the feed served over HTTPS in production. The signature already protects
the payload from tampering, but HTTPS also keeps the game id and version
private. Serve `manifest.json` with a short cache lifetime and the versioned
`.pak` with a long immutable cache.

## Save data

With `--game-id`, saves live in `data_dir()/kaplay-native/<game-id>` on every
platform. Content updates replace the game files but never the save directory,
so `localStorage` survives updates. Games packed without a game id keep using
the shared `data_dir()/kaplay-native` directory for compatibility.

Packing an existing game with a game id for the first time does not migrate the
old shared `storage.json`; copy it into the new directory if those saves matter.

## CI

Build the same web directory you deploy to the web, pack it, upload the pak,
then generate the manifest. A minimal release step looks like this:

```sh
kaplay-native build ./my-game --out dist/game.pak
aws s3 cp dist/game.pak "s3://my-bucket/my-game/$VERSION/game.pak"
kaplay-native manifest dist/game.pak \
  --version "$VERSION" \
  --url "https://cdn.example.com/my-game/$VERSION/game.pak" \
  --game-id my-game \
  --key "$UPDATE_KEY" \
  --out dist/manifest
aws s3 cp dist/manifest/manifest.json "s3://my-bucket/my-game/stable/manifest.json"
aws s3 cp dist/manifest/manifest.json.sig "s3://my-bucket/my-game/stable/manifest.json.sig"
```

Use a secret for the private key (a file written from a CI secret works with
`--key`). Use separate manifest URLs for channels such as `stable` and `beta`;
the game checks only the URL baked in at pack time.

## Steam and itch.io

Steam, the itch app, and other store clients already update installed games. Do
not pack those builds with `--update-url`: the store client owns the install
directory, and a self-updater would fight its file verification. The host also
skips the check when Steam launches the process. Ship updatable content only for
direct downloads.

## Limitations

- A content update that needs new host APIs must wait for a new executable. Use
  `--min-host` so old hosts ignore it instead of failing.
- There is no automatic crash rollback in this version. If new content is
  broken, `--reset-content` returns to the embedded content; the previous `.pak`
  remains in the update directory for manual recovery.
- Content is downloaded and applied on the next launch, not hot-swapped during
  play.
