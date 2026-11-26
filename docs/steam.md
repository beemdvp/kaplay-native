# Steam

Steam support has two sides: the runtime must be hookable by the overlay, and
games need Steamworks features.

## Overlay

The Steam overlay hooks the swap chain of the process. Chromium shells,
webviews, and Electron cannot be hooked reliably, which is why this runtime
owns its window and GL context directly and calls `SwapBuffers` itself. When
`steam_api` is loaded and `SteamAPI_Init` succeeds, the overlay attaches to the
window automatically; there is no code path in KAPLAY Native that draws the
overlay itself.

Two ordering rules come from the Steamworks documentation:

1. `SteamAPI_RestartAppIfNecessary` runs before any window or GL object exists.
   The host calls it with `--appid` and exits when Steam asks for a relaunch.
2. `SteamAPI_Init` runs before the GL context is created so the overlay can
   hook the swap chain from the start.

`SteamAPI_RunCallbacks` runs once per frame after the game's frame work.

## Library loading

Steamworks redistributables are never bundled. At startup the host searches for
`steam_api64.dll` / `steam_api.dll` (Windows), `libsteam_api.so` (Linux), or
`libsteam_api.dylib` (macOS) in this order:

1. the path in `KAPLAY_NATIVE_STEAM_LIB`
2. the directory containing the executable
3. the current working directory

Each flat API symbol is resolved individually. A missing library or symbol
disables Steam integration, logs at debug level, and the game keeps running.
`--no-steam` skips loading entirely, which is what CI uses.

Initialisation uses `SteamAPI_InitFlat` when the library exports it (SDK 1.65
made `SteamAPI_Init` an inline header function) and falls back to
`SteamAPI_Init` for older redistributables. The player name comes from
`ISteamUser::GetPlayerName` when that flat symbol exists (SDK 1.64 and older),
otherwise from `ISteamFriends::GetFriendPersonaName` for the local user.

Interfaces whose symbol names carry a version suffix are requested by the
suffix the current SDK exports:

- `SteamAPI_SteamUser_v023`
- `SteamAPI_SteamUserStats_v013` (falls back to `v012`)
- `SteamAPI_SteamRemoteStorage_v016`
- `SteamAPI_SteamFriends_v018` (loaded only when the player name needs it)

If a future SDK bumps those versions, loading fails soft and the rest of the
runtime is unaffected; update the suffix in `crates/steam/src/lib.rs`.

## Game API

Games reach Steam through the `kaplayNative.steam` namespace:

```js
const steam = kaplayNative.steam;
steam.available();                        // boolean
steam.playerName();                       // string | null
steam.steamId();                          // string (decimal, 64-bit safe)
steam.unlockAchievement("FIRST_WIN");     // boolean
steam.isAchievementUnlocked("FIRST_WIN"); // boolean
steam.storeStats();                       // boolean
steam.setStatInt("wins", 3);              // boolean
steam.getStatInt("wins");                 // number | null
steam.setStatFloat("play_time", 12.5);    // boolean
steam.getStatFloat("play_time");          // number | null
steam.writeCloud("save.json", "{}");      // boolean
steam.readCloud("save.json");             // string | null
steam.cloudExists("save.json");           // boolean
steam.deleteCloud("save.json");           // boolean
steam.runCallbacks();                     // returns undefined
```

Every call returns a safe fallback when Steam is unavailable, so games can
call the API unconditionally. Achievement unlocks and stat writes commit
`StoreStats` immediately. Cloud files are written through the remote storage
API and are subject to Steam's per-file size limits.

The host calls `runCallbacks` every frame already; the JavaScript method exists
for games that load Steam state from their own event loops or tests.

## Packaging

`kaplay-native build` packs assets, not Steam binaries. For a Steam depot:

1. Place `steam_api64.dll` / `libsteam_api.so` next to the executable.
2. Ship `steam_appid.txt` only in development builds. Never ship it in a depot
   release: Steam sets the app id from the launch context.
3. Pass the app id in production with `--appid <id>` (or the CLI equivalent)
   so `RestartAppIfNecessary` works.

## Local testing without a depot

1. Start the Steam client and sign in.
2. Put a `steam_appid.txt` containing `480` (Spacewar) in the working
   directory. Do not pass `--appid` in this mode: the flat API reads the app id
   from the file, and `RestartAppIfNecessary` would try to relaunch an app the
   account does not own.
3. Point the runtime at the SDK redistributable either with
   `KAPLAY_NATIVE_STEAM_LIB=/path/to/sdk/redistributable_bin/linux64/libsteam_api.so`
   or by copying `libsteam_api.so` next to the host binary.
4. Run the host from the directory with `steam_appid.txt`. Achievements,
   stats, and cloud files persist per Steam user and can be re-read on the
   next run.

`scripts/steam-overlay-test.sh` automates those steps, including the direct
launch preload. It checks the client is running, resolves the library from
`KAPLAY_NATIVE_STEAM_LIB` or a local SDK copy, writes `steam_appid.txt`, and
runs the bridge example unless host arguments are passed after `--`. Use
`--check` to validate the setup without launching, or `--demo` to run
`examples/steam`, an on-screen demo of the API: it shows player, stat,
achievement, and cloud status, and binds Space (cloud write), A (unlock
`ACH_WIN_ONE_GAME`), S (add a `NumWins` stat and store), and D (cloud delete).
The demo needs a KAPLAY bundle, resolved from `$KAPLAY_BUNDLE` or a local
kaplay checkout, and expects app id 480 because it uses Spacewar's achievement
and stat names.

On a Wayland session the script passes `--x11` to the host: Steam's overlay
renderer loads but never attaches to a native Wayland window, while it hooks
the GLX context of the XWayland window right away. If you pass your own host
arguments after `--`, remember to include `--x11` there too.

To exercise the `RestartAppIfNecessary` branch without launching from the
client, pass `--appid 480` together with `SteamAppId=480 SteamGameId=480` in
the environment: the call then sees a Steam launch context and returns false.

The overlay only hooks when Steam injects `gameoverlayrenderer.so`. Launching
through the client is the real test; for a direct launch,
`LD_PRELOAD=$HOME/.local/share/Steam/ubuntu12_64/gameoverlayrenderer.so` gives
the renderer a chance to attach.

## Continuous integration

The `Steamworks SDK` CI job (Linux only) and `cargo test -p kaplay-native-steam`
on Linux x86_64 test against the vendored redistributable
`crates/steam/tests/fixtures/libsteam_api.so`. The fixture is test-only: it is
never linked into the host or packed into a game, so the "never bundle the
redistributables" rule for distribution still holds.

`scripts/steam-api-test.sh` runs `scripts/steam-api-test.js` under every engine
the host binary provides and checks:

- `--no-steam` and a missing library: safe fallbacks, the game keeps rendering
- the real redistributable: every flat API symbol resolves, `InitFlat` fails
  soft without a client, and `RestartAppIfNecessary` does not relaunch when a
  Steam launch context is present
- the documented JS contract on both engines, including `null` (not
  `undefined`) from `playerName`, `getStatInt`, `getStatFloat`, and `readCloud`

CI runners have no Steam client, so live stats, achievements, cloud, and the
overlay stay local/manual tests. The SDK download requires a partner login, so
CI cannot fetch new releases itself: refresh the fixture when Valve ships a
new SDK (procedure in `crates/steam/tests/fixtures/README.md`). A Valve-side
interface version bump fails CI once the fixture is refreshed.

## Manual validation checklist

Overlay behavior cannot be verified without a Steam client, so it stays a
manual test. Run it on each platform before a release:

- [ ] Launch from the Steam library. The overlay appears on Shift+Tab.
- [ ] Launch from outside Steam. `RestartAppIfNecessary` relaunches through
      Steam (or exits cleanly when no Steam client is running).
- [ ] The overlay draws over the game and the game keeps rendering behind it.
- [ ] Screenshots (F12) and the in-overlay web browser work.
- [ ] Achievements unlock while the overlay is open and appear in the client.
- [ ] Cloud writes from one machine appear on another.
- [ ] Alt-tab, window resize, and fullscreen toggles do not detach the overlay.
- [ ] The game still runs with `--no-steam` and with the library removed.
