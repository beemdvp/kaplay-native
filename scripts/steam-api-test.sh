#!/usr/bin/env bash
# Steamworks API regression test.
#
# Runs scripts/steam-api-test.js under every JS engine the host binary
# provides, across the offline scenarios and against the vendored Steamworks
# redistributable (crates/steam/tests/fixtures/libsteam_api.so). CI runs this
# on Linux without a Steam client, so the library cases exercise loading,
# symbol resolution, and the soft-failure path; with a client running they
# exercise the live API instead.
#
# usage: scripts/steam-api-test.sh [--bin <host>] [--lib <libsteam_api>]
#
# The host binary must already be built with both engines for full coverage:
#   cargo build --release -p kaplay-native-host --features v8
# Engines the binary lacks are skipped with a notice. On a headless machine
# wrap the call in xvfb-run and set LIBGL_ALWAYS_SOFTWARE=1, as CI does.
set -uo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(dirname "$script_dir")"

bin="$repo_root/target/release/kaplay-native-host"
lib="$repo_root/crates/steam/tests/fixtures/libsteam_api.so"

usage() {
    sed -n '2,16p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}
die() {
    echo "steam-api-test: $*" >&2
    exit 1
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --bin)
            [[ $# -ge 2 ]] || die "--bin needs a value"
            bin="$2"; shift 2 ;;
        --lib)
            [[ $# -ge 2 ]] || die "--lib needs a value"
            lib="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown option: $1 (try --help)" ;;
    esac
done

[[ -x "$bin" ]] || die "host binary not found: $bin
build it with: cargo build --release -p kaplay-native-host --features v8"
[[ -f "$lib" ]] || die "steam library not found: $lib"

# cases run from a temp workdir, so resolve relative paths now
bin="$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")"
lib="$(cd "$(dirname "$lib")" && pwd)/$(basename "$lib")"

engines="$("$bin" --engines)" || die "cannot list engines of $bin"

total=0
failed=0

workdir="$(mktemp -d)"
trap '[[ $failed -eq 0 ]] && rm -rf "$workdir"' EXIT
mkdir -p "$workdir/logs"
cp "$script_dir/steam-api-test.js" "$workdir/"
printf '480\n' > "$workdir/steam_appid.txt"

export RUST_LOG="${RUST_LOG:-info}"

# Steam's overlay and the GL path are happiest on X11; match the overlay
# script's Wayland fallback. Under CI's xvfb neither variable is set.
x11_args=()
if [[ "$(uname -s)" == Linux && ( "${XDG_SESSION_TYPE:-}" == wayland || -n "${WAYLAND_DISPLAY:-}" ) ]]; then
    x11_args=(--x11)
fi

client_up=0
pgrep -x steam >/dev/null 2>&1 && client_up=1
libmode=offline
[[ $client_up == 1 ]] && libmode=live
echo "steam-api-test: client running: $client_up (library cases expect: $libmode)"
echo "steam-api-test: host $bin"
echo "steam-api-test: lib  $lib"

# run_case <name> <expected-mode> [env args...] -- [host args...]
# engine: -v8 suffix -> v8 (default), -qjs suffix -> quickjs
run_case() {
    local name="$1" mode="$2"; shift 2
    local envs=()
    while [[ $# -gt 0 && "$1" != "--" ]]; do envs+=("$1"); shift; done
    [[ $# -gt 0 ]] && shift
    local log="$workdir/logs/$name.log"
    local js=v8; [[ "$name" == *-qjs ]] && js=quickjs
    local attempt rc summary fails gotmode
    for attempt in 1 2; do
        (
            cd "$workdir" &&
            env "${envs[@]}" "$bin" \
                --script "$workdir/steam-api-test.js" --dir "$workdir" \
                --window 320x240 --frames 20 --no-audio --headless \
                --js-engine "$js" "${x11_args[@]}" "$@" \
                >"$log" 2>&1
        )
        rc=$?
        summary=$(grep -o 'STEAMTEST SUMMARY pass=[0-9]* fail=[0-9]* mode=[a-z]* frames=[0-9]*' "$log" | tail -1)
        fails=$(sed -n 's/.*fail=\([0-9]*\).*/\1/p' <<<"$summary")
        gotmode=$(sed -n 's/.*mode=\([a-z]*\).*/\1/p' <<<"$summary")
        if [[ $rc -eq 0 && -n "$summary" && "$fails" == "0" && "$gotmode" == "$mode" ]]; then
            break
        fi
        # With a live client, Steam Cloud sync churn on Spacewar can reject
        # writes briefly; let the client settle and retry once.
        [[ $attempt == 1 ]] && sleep 6
    done
    [[ $client_up == 1 ]] && sleep 2
    total=$((total + 1))
    if [[ $rc -eq 0 && -n "$summary" && "$fails" == "0" && "$gotmode" == "$mode" ]]; then
        echo "CASE PASS  $name  ($summary)"
    else
        failed=$((failed + 1))
        echo "CASE FAIL  $name  rc=$rc expected-mode=$mode summary='${summary:-none}'"
        grep -E 'STEAMTEST FAIL|panicked|Assertion' "$log" | head -15
        echo "  full log: $log"
    fi
}

offline_env=(-u KAPLAY_NATIVE_STEAM_LIB -u SteamAppId -u SteamGameId)

for engine in v8 quickjs; do
    grep -qx "$engine" <<<"$engines" || {
        echo "SKIP $engine cases: not compiled into $bin"
        continue
    }
    suffix=qjs; [[ $engine == v8 ]] && suffix=v8

    # loading skipped entirely
    run_case "no-steam-$suffix" offline "${offline_env[@]}" -- --no-steam

    # library absent everywhere: soft failure, game keeps running
    if [[ -f "$(dirname "$bin")/libsteam_api.so" || -f "$(dirname "$bin")/steam_api64.dll" ]]; then
        echo "SKIP missing-lib-$suffix: a steam library sits next to the host binary"
    else
        run_case "missing-lib-$suffix" offline "${offline_env[@]}" --
    fi

    # real redistributable: init follows client state
    run_case "lib-$suffix" "$libmode" \
        -u SteamAppId -u SteamGameId "KAPLAY_NATIVE_STEAM_LIB=$lib" --
    if [[ $client_up == 0 ]]; then
        grep -q 'steam library loaded (client running: false)' "$workdir/logs/lib-$suffix.log" ||
            echo "  NOTE: expected 'client running: false' log missing ($engine)"
        grep -q 'steam unavailable' "$workdir/logs/lib-$suffix.log" ||
            echo "  NOTE: expected 'steam unavailable' warning missing ($engine)"
    else
        grep -q 'steam initialised' "$workdir/logs/lib-$suffix.log" ||
            echo "  NOTE: expected 'steam initialised' log missing ($engine)"
    fi

    # RestartAppIfNecessary sees a Steam launch context -> must NOT relaunch
    run_case "restart-branch-$suffix" "$libmode" \
        "KAPLAY_NATIVE_STEAM_LIB=$lib" SteamAppId=480 SteamGameId=480 -- --appid 480
    grep -q 'relaunching through Steam' "$workdir/logs/restart-branch-$suffix.log" &&
        echo "  NOTE: unexpected relaunch in restart-branch-$suffix"
done

echo "----------------------------------------"
echo "steam-api-test: $total cases, $failed failed"
[[ $failed -eq 0 ]]