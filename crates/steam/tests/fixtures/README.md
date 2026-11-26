# Steamworks test fixture

`libsteam_api.so` is the Linux x86_64 redistributable from the Steamworks SDK,
vendored **for tests only**. It is never linked into the host, bundled into a
pak, or shipped with a game: the runtime still loads Steam dynamically at run
time (see `docs/steam.md`).

| | |
|---|---|
| SDK version | v1.65 (23 July 2026) |
| Source | `sdk/redistributable_bin/linux64/libsteam_api.so` |
| SHA-256 | `eb2dd015b84177cf4f4326fe578aab375fd8931bbbd719c7492420d9777007fe` |

The Steamworks SDK download requires a partner login, so CI cannot fetch new
releases itself. The `Steamworks SDK` CI job and `cargo test -p
kaplay-native-steam` on Linux x86_64 test against this file; a Valve-side
interface version bump therefore fails CI only after the fixture is refreshed.

## Refreshing to a newer SDK

1. Download the current SDK from <https://partner.steamgames.com/downloads/list>.
2. Copy `redistributable_bin/linux64/libsteam_api.so` over this file.
3. Update the version, date, and SHA-256 above (`sha256sum libsteam_api.so`).
4. Run `cargo test -p kaplay-native-steam` and `scripts/steam-api-test.sh`
   locally; if Valve renamed a versioned interface symbol, update the suffix in
   `crates/steam/src/lib.rs` and the note in `docs/steam.md`.

## License

The files under `redistributable_bin` are the redistributable portion of the
Steamworks SDK and may be distributed in object code form under the Steamworks
SDK license. This copy exists solely as a test fixture in the source repository.