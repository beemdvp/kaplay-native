//! Integration tests against the vendored Steamworks redistributable
//! (`tests/fixtures/libsteam_api.so`, see the fixture README for the SDK
//! version and refresh procedure).
//!
//! Linux x86_64 only: the fixture is a Linux x86_64 ELF. On other platforms
//! this file compiles to an empty test binary.

#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::path::PathBuf;

use kaplay_native_steam::{SteamApi, SteamError};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("libsteam_api.so")
}

#[test]
fn real_sdk_exports_every_symbol_the_crate_resolves() {
    let path = fixture();
    assert!(path.is_file(), "fixture missing: {}", path.display());
    let api = SteamApi::from_library(&path).expect("all flat API symbols must resolve");
    // Pre-init fallbacks are safe regardless of client state.
    assert!(!api.is_initialized());
    assert_eq!(api.steam_id(), 0);
    assert!(api.player_name().is_none());
    assert!(!api.unlock_achievement("KAPLAY_NATIVE_TEST"));
    assert!(!api.is_achievement_unlocked("KAPLAY_NATIVE_TEST"));
    assert!(!api.store_stats());
    assert!(api.get_stat_int("KAPLAY_NATIVE_TEST").is_none());
    assert!(api.get_stat_float("KAPLAY_NATIVE_TEST").is_none());
    assert!(api.read_file("KAPLAY_NATIVE_TEST").is_none());
    assert!(!api.file_exists("KAPLAY_NATIVE_TEST"));
}

#[test]
fn init_is_a_soft_failure_without_a_steam_client() {
    let mut api = SteamApi::from_library(&fixture()).expect("fixture loads");
    // CI has no Steam client: init must return a clean Init error, never
    // panic. A dev machine may have a client running, so Ok is also valid.
    match api.init() {
        Ok(()) => assert!(api.is_initialized()),
        Err(SteamError::Init(message)) => {
            assert!(!api.is_initialized());
            assert!(!message.is_empty());
        }
        Err(other) => panic!("unexpected init error: {other}"),
    }
    api.shutdown();
    assert!(!api.is_initialized());
}

#[test]
fn non_library_file_is_rejected() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("README.md");
    assert!(matches!(
        SteamApi::from_library(&path),
        Err(SteamError::LibraryMissing)
    ));
}
