#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

case "$(uname -s)" in
    Linux*) HOST_OS=linux ;;
    Darwin*) HOST_OS=macos ;;
    MINGW*|MSYS*|CYGWIN*) HOST_OS=windows ;;
    *)
        echo "unsupported host: $(uname -s)" >&2
        exit 1
        ;;
esac

if [ "$HOST_OS" = windows ]; then
    LIB=librusty_v8.lib
else
    LIB=librusty_v8.a
fi

TARGET_DIR=${CARGO_TARGET_DIR:-target}
TARGET=${TARGET:-}
HOST=$(rustc -vV | sed -n 's/^host: //p')

if [ -n "$TARGET" ] && [ "$TARGET" != "$HOST" ]; then
    BUILD_ARGS=(--target "$TARGET")
    RELEASE_DIR=$TARGET_DIR/$TARGET/release
else
    BUILD_ARGS=()
    RELEASE_DIR=$TARGET_DIR/release
fi

OUT=${1:-$TARGET_DIR/slim-v8/$LIB}
CACHE=${KAPLAY_CACHE_DIR:-$HOME/.cache/kaplay-native}
LLVM_VERSION=23.1.3

if [ "$HOST_OS" != linux ]; then
    # binding.cc includes ICU headers even with v8_enable_i18n_support=false.
    # Linux finds them in the system include path; point the other hosts at
    # the ICU sources vendored with the v8 crate instead.
    cargo fetch --locked
    V8_VERSION=$(awk -F'"' '/^name = "v8"$/{getline; print $2; exit}' Cargo.lock)
    V8_DIR=""
    for dir in "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/v8-"$V8_VERSION"; do
        if [ -d "$dir" ]; then
            V8_DIR=$dir
            break
        fi
    done
    if [ -z "$V8_DIR" ]; then
        echo "could not find the extracted v8-$V8_VERSION crate" >&2
        exit 1
    fi
    ICU_SRC=$V8_DIR/third_party/icu/source
    if [ "$HOST_OS" = windows ]; then
        ICU_PATH="$(cygpath -w "$ICU_SRC/common");$(cygpath -w "$ICU_SRC/i18n")"
        export CPATH="$ICU_PATH"
        export INCLUDE="$ICU_PATH${INCLUDE:+;$INCLUDE}"
    else
        export CPATH="$ICU_SRC/common:$ICU_SRC/i18n${CPATH:+:$CPATH}"
    fi
    echo "ICU headers for binding.cc: $ICU_SRC"
fi

if [ -z "${JOBS:-}" ]; then
    case "$HOST_OS" in
        linux) JOBS=$(nproc 2>/dev/null || echo 4) ;;
        macos) JOBS=$(sysctl -n hw.ncpu 2>/dev/null || echo 4) ;;
        windows) JOBS=${NUMBER_OF_PROCESSORS:-4} ;;
    esac
fi

mkdir -p "$CACHE"

if [ "$HOST_OS" = linux ]; then
    case "$(uname -m)" in
        x86_64) GN_PKG=linux-amd64; LLVM_ASSET=Linux-X64 ;;
        aarch64) GN_PKG=linux-arm64; LLVM_ASSET=Linux-ARM64 ;;
        *)
            echo "unsupported architecture: $(uname -m)" >&2
            exit 1
            ;;
    esac

    if [ -z "${GN:-}" ] && ! command -v gn >/dev/null 2>&1; then
        GN_DIR=$CACHE/gn-$GN_PKG
        if [ ! -x "$GN_DIR/gn" ]; then
            echo "downloading gn"
            curl -fsSL -o "$CACHE/gn-$GN_PKG.zip" \
                "https://chrome-infra-packages.appspot.com/dl/gn/gn/$GN_PKG/+/latest"
            mkdir -p "$GN_DIR"
            unzip -oq "$CACHE/gn-$GN_PKG.zip" -d "$GN_DIR"
            chmod +x "$GN_DIR/gn"
            rm -f "$CACHE/gn-$GN_PKG.zip"
        fi
        export GN=$GN_DIR/gn
    fi

    if [ -z "${LIBCLANG_PATH:-}" ] && [ -z "${CLANG_BASE_PATH:-}" ]; then
        for base in /usr/lib/llvm-2* /usr/lib/llvm-3*; do
            [ -x "$base/bin/clang" ] || continue
            if ls "$base"/lib/libclang.so.2[1-9]* "$base"/lib/libclang.so.3* >/dev/null 2>&1; then
                export LIBCLANG_PATH=$base/lib
                export CLANG_BASE_PATH=$base
                break
            fi
        done
    fi

    if [ -z "${LIBCLANG_PATH:-}" ] || [ -z "${CLANG_BASE_PATH:-}" ]; then
        LLVM_DIR=$CACHE/llvm-$LLVM_VERSION
        if [ ! -e "$LLVM_DIR/lib/libclang.so" ] || [ ! -x "$LLVM_DIR/bin/clang" ]; then
            echo "downloading LLVM $LLVM_VERSION (libclang 21+ and a native clang; Chromium's download is x64-only)"
            mkdir -p "$LLVM_DIR"
            LLVM_MAJOR=${LLVM_VERSION%%.*}
            if command -v zstd >/dev/null 2>&1; then
                curl -fsSL -o "$CACHE/llvm.tar.zst" \
                    "https://github.com/llvm/llvm-project/releases/download/llvmorg-$LLVM_VERSION/LLVM-$LLVM_VERSION-$LLVM_ASSET.tar.zst"
                zstd -d --long=30 -c "$CACHE/llvm.tar.zst" |
                    tar -x -C "$LLVM_DIR" --strip-components=1 --wildcards \
                        '*/lib/*' '*/bin/clang' '*/bin/clang++' "*/bin/clang-$LLVM_MAJOR" \
                        '*/bin/llvm-*' '*/bin/lld' '*/bin/ld.lld'
                rm -f "$CACHE/llvm.tar.zst"
            else
                curl -fsSL -o "$CACHE/llvm.tar.xz" \
                    "https://github.com/llvm/llvm-project/releases/download/llvmorg-$LLVM_VERSION/LLVM-$LLVM_VERSION-$LLVM_ASSET.tar.xz"
                tar -xJf "$CACHE/llvm.tar.xz" -C "$LLVM_DIR" --strip-components=1 --wildcards \
                    '*/lib/*' '*/bin/clang' '*/bin/clang++' "*/bin/clang-$LLVM_MAJOR" \
                    '*/bin/llvm-*' '*/bin/lld' '*/bin/ld.lld'
                rm -f "$CACHE/llvm.tar.xz"
            fi
        fi
        export LIBCLANG_PATH=${LIBCLANG_PATH:-$LLVM_DIR/lib}
        export CLANG_BASE_PATH=${CLANG_BASE_PATH:-$LLVM_DIR}
    fi
fi

if [ "$HOST_OS" = macos ] && [ -z "${LIBCLANG_PATH:-}" ]; then
    if ! brew list --versions llvm@21 >/dev/null 2>&1; then
        echo "installing llvm@21 for libclang (bindgen needs libclang 21+)"
        brew install llvm@21
    fi
    export LIBCLANG_PATH="$(brew --prefix llvm@21)/lib"
fi

if [ -n "${CLANG_BASE_PATH:-}" ] && [ -z "${RUSTY_V8_BINDGEN_RESOURCE_DIR:-}" ]; then
    export RUSTY_V8_BINDGEN_RESOURCE_DIR=$("$CLANG_BASE_PATH/bin/clang" -print-resource-dir)
fi

export NUM_JOBS=$JOBS
export V8_FROM_SOURCE=1
export GN_ARGS="${GN_ARGS:+$GN_ARGS }v8_enable_i18n_support=false v8_enable_webassembly=false v8_enable_temporal_support=false use_glib=false"

# build.rs only relaxes -Werror when a system clang is configured, which the
# Linux path always does; keep the other hosts on the same GN config.
if [ "$HOST_OS" != linux ] && [ -z "${CLANG_BASE_PATH:-}" ]; then
    export GN_ARGS="$GN_ARGS treat_warnings_as_errors=false"
fi

# GN defaults target_cpu to host_cpu, and build.rs pins host_cpu to arm64 on
# Apple Silicon, so an x86_64 macOS build on an arm64 runner needs it back.
if [ "${TARGET:-$HOST}" = x86_64-apple-darwin ]; then
    export EXTRA_GN_ARGS="${EXTRA_GN_ARGS:+$EXTRA_GN_ARGS }target_cpu=\"x64\""
fi

if [ "$HOST_OS" = linux ] && [ -z "${RUSTFLAGS:-}" ]; then
    for dir in "$HOME"/.rustup/toolchains/*/lib/rustlib/*/bin/gcc-ld; do
        if [ -x "$dir/ld.lld" ]; then
            export PATH=$dir:$PATH
            break
        fi
    done
    if command -v ld.lld >/dev/null 2>&1; then
        export RUSTFLAGS="-C force-unwind-tables=no -C link-arg=-fuse-ld=lld -C link-arg=-Wl,--icf=all"
    fi
fi

echo "building slim V8 without i18n, WebAssembly and Temporal (jobs=$JOBS)"
cargo build --release ${BUILD_ARGS[@]+"${BUILD_ARGS[@]}"} -p kaplay-native-host --no-default-features --features v8

ARCHIVE=$RELEASE_DIR/gn_out/obj/$LIB
mkdir -p "$(dirname "$OUT")"
cp "$ARCHIVE" "$OUT"
echo "slim V8 archive: $OUT"
echo "rebuild any host against it with RUSTY_V8_ARCHIVE=$OUT"
