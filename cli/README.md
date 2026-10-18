# kaplay-native

Run and package [KAPLAY](https://kaplayjs.com) games as native desktop
applications on Linux, Windows, and macOS.

```sh
npm install --global kaplay-native

kaplay-native run ./my-game
kaplay-native pack ./my-game --out dist/my-game --js-engine v8
kaplay-native keygen --out update-key.pem
kaplay-native doctor
```

`run` executes a game with the native host. `pack` writes a single-file
executable for the current platform with the game assets and defaults embedded,
so players need nothing else. The CLI downloads the matching native host from
GitHub releases on first use and caches it.

Packed games can also update their content without replacing the executable.
`keygen` creates an Ed25519 key pair, `pack --update-url --update-key` embeds the
feed and public key, and `manifest` signs the published `.pak`. Games can poll
`kaplayNative.updates.status()` and restart into an update with
`kaplayNative.updates.apply()`. The full workflow is in
[docs/updates.md](https://github.com/beemdvp/kaplay-native/blob/main/docs/updates.md).

The full documentation, including the packed format and a copy-paste GitHub
Actions workflow for shipping to all platforms, lives in
[docs/distribution.md](https://github.com/beemdvp/kaplay-native/blob/main/docs/distribution.md).
