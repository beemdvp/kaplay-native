#!/usr/bin/env node
import { spawn, spawnSync } from "node:child_process";
import { createHash, createPrivateKey, generateKeyPairSync, sign } from "node:crypto";
import {
    chmodSync,
    copyFileSync,
    existsSync,
    mkdirSync,
    readdirSync,
    readFileSync,
    statSync,
    watch,
    writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { findLocalHost } from "./host.mjs";
import { isMachO, isPe, patchWindowsIcon, prepareIcon } from "./icon.mjs";
import { createZip } from "./zip.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "..", "..");
const pkg = JSON.parse(readFileSync(join(here, "..", "package.json"), "utf8"));
const GITHUB_REPO = "beemdvp/kaplay-native";
const hostName = process.platform === "win32" ? "kaplay-native-host.exe" : "kaplay-native-host";
const TRAILER_MAGIC = "KNATIVEP";
const TRAILER_VERSION = 1;
const SEMVER = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;
const GAME_ID = /^[A-Za-z0-9._-]{1,96}$/;

function platformTag() {
    const tags = {
        "linux-x64": "linux-x64",
        "linux-arm64": "linux-arm64",
        "darwin-x64": "macos-x64",
        "darwin-arm64": "macos-arm64",
        "win32-x64": "windows-x64",
    };
    return tags[`${process.platform}-${process.arch}`] ?? null;
}

function hostAssetName(engine) {
    const tag = platformTag();
    if (!tag) return null;
    const suffix = engine === "v8" ? "-v8" : "";
    return `kaplay-native-host-${tag}${suffix}${tag.startsWith("windows") ? ".exe" : ""}`;
}

function cacheDir() {
    if (process.env.KAPLAY_NATIVE_CACHE) {
        return join(process.env.KAPLAY_NATIVE_CACHE, pkg.version);
    }
    if (process.platform === "win32") {
        const base = process.env.LOCALAPPDATA ?? join(homedir(), "AppData", "Local");
        return join(base, "kaplay-native", pkg.version);
    }
    if (process.platform === "darwin") {
        return join(homedir(), "Library", "Caches", "kaplay-native", pkg.version);
    }
    const base = process.env.XDG_CACHE_HOME ?? join(homedir(), ".cache");
    return join(base, "kaplay-native", pkg.version);
}

function usage() {
    console.log(`kaplay-native <command> [options]

Commands:
  run [dir]         Run a game from a directory
  dev [dir]         Run and restart on file changes
  build [dir]       Bundle a game and pack it into a .pak
  pack [dir]        Build a game into a single-file executable for this platform
  test [dir]        Run a game headlessly and compare a screenshot
  keygen            Generate an Ed25519 key pair for content updates
  manifest <pak>    Write a signed content manifest for a .pak
  engines           Print the compiled-in javascript engines
  doctor            Check the local toolchain

Packaged binaries:
  pack writes <dir> assets and a small config into a copy of the host binary,
  so the result runs on its own with no extra files.

Common options:
  --host <path>     Path to the kaplay-native-host binary
  --pak <path>      Run or read a .pak archive
  --entry <file>    Game entry script (default: main.js)
  --web-dist <dir>  Run a web build directory (Vite/Rollup output, uses its index.html entry)
  --preload <file>  Script evaluated before the entry (repeatable)
  --window <WxH>    Window size (default: 960x540)
  --frames <n>      Exit after n frames
  --screenshot <p>  Write a screenshot on exit
  --out <path>      Output path for build and pack
  --headless        Do not show a window
  --fullscreen      Start fullscreen
  --title <name>    Window title
  --allow-network   Allow network requests (disabled by default)
  --no-audio        Run without audio output
  --no-vsync        Disable vertical sync (uncapped frame rate)
  --stats           Log the frame rate while the game runs
  --bench-json <p>  Write a JSON benchmark summary before exiting
  --frame-trace <p> Write per-frame timings as JSON before exiting
  --url <url>       Page URL games see as window.location (resolves relative requests)
  --js-engine <n>   Javascript engine to run or embed (quickjs or v8)
  --x11             Prefer the X11 backend on Linux
  --no-update       Skip the content update check
  --reset-content   Delete installed content updates before starting
  --icon <png>      Square PNG for the window, Windows exe, and macOS icns

Content update options:
  --game-id <id>        Stable id for saves and updates
  --content-version <v> Version of the packaged content (semver)
  --update-url <url>    Manifest URL checked on launch
  --update-key <key>    Ed25519 public key (repeatable, base64 or a file)

Manifest options:
  --version <v>     Content version (semver)
  --url <url>       Download URL for the pak
  --key <file>      Ed25519 private key from keygen
  --min-host <v>    Minimum host version required by the content
  --rollout <n>     Percentage of installs that receive the content (0-100)
  --required        Mark the update as required by the game
  --out <path>      Output directory or manifest.json path
`);
}

function parseArgs(argv) {
    const options = { preload: [], _: [] };
    for (let i = 0; i < argv.length; i++) {
        const arg = argv[i];
        if (!arg.startsWith("--")) {
            options._.push(arg);
            continue;
        }
        const key = arg.slice(2);
        const next = argv[i + 1];
        const takesValue = !["headless", "fullscreen", "allow-network", "no-audio", "no-vsync", "stats", "x11", "help", "update", "no-update", "reset-content", "force", "required"].includes(key);
        if (takesValue && next !== undefined && !next.startsWith("--")) {
            if (key === "preload") options.preload.push(next);
            else if (key === "update-key") {
                if (!Array.isArray(options["update-key"])) options["update-key"] = [];
                options["update-key"].push(next);
            } else options[key] = next;
            i++;
        } else {
            options[key] = true;
        }
    }
    return options;
}

function which(binary) {
    const result = spawnSync(process.platform === "win32" ? "where" : "which", [binary], {
        encoding: "utf8",
    });
    return result.status === 0 ? binary : null;
}

function fail(message) {
    console.error(message);
    process.exit(1);
}

function requireValue(options, key, label) {
    const value = options[key];
    if (!value || value === true) fail(`missing ${label}`);
    return value;
}

function requireSemver(value, label) {
    if (!SEMVER.test(value)) fail(`${label} must be a semver version, got ${value}`);
    return value;
}

function requireGameId(value) {
    if (!GAME_ID.test(value) || value === "." || value === "..") {
        fail(`game id must match ${GAME_ID.source}, got ${value}`);
    }
    return value;
}

function requireRollout(value) {
    if (value === true) fail("missing --rollout value");
    const rollout = Number(value);
    if (!Number.isInteger(rollout) || rollout < 0 || rollout > 100) {
        fail(`--rollout must be an integer from 0 to 100, got ${value}`);
    }
    return rollout;
}

function requireHttpUrl(value, label) {
    let url;
    try {
        url = new URL(value);
    } catch {
        fail(`${label} must be a URL, got ${value}`);
    }
    if (url.protocol !== "https:" && url.protocol !== "http:") {
        fail(`${label} must be an http(s) URL, got ${value}`);
    }
    return value;
}

function slugify(value) {
    return value
        .toLowerCase()
        .replace(/[^a-z0-9._-]+/g, "-")
        .replace(/^[.-]+|[.-]+$/g, "")
        .slice(0, 96);
}

function loadIcon(value) {
    const path = resolve(value);
    if (!existsSync(path)) fail(`icon not found: ${path}`);
    try {
        return prepareIcon(readFileSync(path));
    } catch (error) {
        fail(`--icon ${value}: ${error.message}`);
    }
}

function readPublicKey(value) {
    if (typeof value !== "string") fail("missing --update-key value");
    const file = resolve(value);
    const text = existsSync(file) && statSync(file).isFile() ? readFileSync(file, "utf8") : value;
    const raw = Buffer.from(text.trim(), "base64");
    if (raw.length !== 32) {
        fail(`update key must be a 32-byte Ed25519 public key in base64: ${value}`);
    }
    return raw.toString("base64");
}

async function downloadHost(engine) {
    const asset = hostAssetName(engine);
    if (!asset) {
        console.error(`no prebuilt host for ${process.platform}-${process.arch}`);
        return null;
    }
    const url = `https://github.com/${GITHUB_REPO}/releases/download/v${pkg.version}/${asset}`;
    console.log(`downloading ${url}`);
    try {
        const response = await fetch(url, { redirect: "follow" });
        if (!response.ok) {
            console.error(`failed to download the host binary: HTTP ${response.status} from ${url}`);
            return null;
        }
        const bytes = Buffer.from(await response.arrayBuffer());
        const dest = join(cacheDir(), asset);
        mkdirSync(dirname(dest), { recursive: true });
        writeFileSync(dest, bytes);
        if (process.platform !== "win32") chmodSync(dest, 0o755);
        return dest;
    } catch (error) {
        console.error(`failed to download the host binary: ${error.message}`);
        return null;
    }
}

async function findHost(explicit, engine) {
    const candidates = [explicit, process.env.KAPLAY_NATIVE_HOST].filter(Boolean);
    for (const candidate of candidates) {
        if (existsSync(candidate) && statSync(candidate).isFile()) return resolve(candidate);
        if (!candidate.includes("/") && !candidate.includes("\\") && which(candidate)) {
            return candidate;
        }
    }
    const local = findLocalHost([
        join(repoRoot, "target", "release", hostName),
        join(repoRoot, "target", "debug", hostName),
    ], engine);
    if (local) return local;
    const onPath = which(hostName);
    if (onPath) return onPath;
    const asset = hostAssetName(engine);
    const cached = asset ? join(cacheDir(), asset) : null;
    if (cached && existsSync(cached)) return cached;
    return downloadHost(engine);
}

function hostEngines(host) {
    const result = spawnSync(host, ["--engines"], { encoding: "utf8" });
    if (result.status !== 0) return null;
    return result.stdout.trim().split(/\s+/).filter(Boolean);
}

async function requireHost(explicit, engine) {
    const host = await findHost(explicit, engine);
    if (!host) {
        console.error("kaplay-native-host not found. Build it with `cargo build --release`, pass --host <path>, or let the CLI download a release.");
        process.exit(1);
    }
    return host;
}

function hostArgs(options, extra = {}) {
    const args = [];
    if (options.pak) args.push("--pak", options.pak);
    if (extra.dir) args.push("--dir", extra.dir);
    if (extra.script) args.push("--script", extra.script);
    for (const preload of options.preload) args.push("--preload", preload);
    if (options.window) args.push("--window", options.window);
    if (options.frames) args.push("--frames", String(options.frames));
    if (options.screenshot) args.push("--screenshot", options.screenshot);
    if (options.title) args.push("--title", options.title);
    if (options.appid) args.push("--appid", String(options.appid));
    if (options.fullscreen) args.push("--fullscreen");
    if (options.headless) args.push("--headless");
    if (options["allow-network"]) args.push("--allow-network");
    if (options["no-audio"]) args.push("--no-audio");
    if (options["no-vsync"]) args.push("--no-vsync");
    if (options.stats) args.push("--stats");
    if (options["bench-json"]) args.push("--bench-json", options["bench-json"]);
    if (options["frame-trace"]) args.push("--frame-trace", options["frame-trace"]);
    if (options.url) args.push("--url", options.url);
    if (options["js-engine"]) args.push("--js-engine", options["js-engine"]);
    if (options.x11) args.push("--x11");
    if (options["no-update"]) args.push("--no-update");
    if (options["reset-content"]) args.push("--reset-content");
    return args;
}

function gameDir(options) {
    const dir = resolve(options["web-dist"] ?? options._[0] ?? ".");
    if (!existsSync(dir)) {
        console.error(`game directory not found: ${dir}`);
        process.exit(1);
    }
    return dir;
}

async function withHost(options, args, { capture = false } = {}) {
    const host = await requireHost(options.host, options["js-engine"]);
    if (capture) {
        return spawnSync(host, args, { encoding: "utf8" });
    }
    const child = spawn(host, args, { stdio: "inherit" });
    child.on("exit", (code) => process.exit(code ?? 0));
    return child;
}

function autoPreload(dir, options) {
    if (options.preload.length) return options;
    const candidate = join(dir, "kaplay.js");
    if (existsSync(candidate)) {
        return { ...options, preload: [candidate] };
    }
    return options;
}

async function runCommand(options) {
    const dir = gameDir(options);
    await withHost(autoPreload(dir, options), hostArgs(options, { dir }));
}

async function devCommand(options) {
    const dir = gameDir(options);
    const base = autoPreload(dir, options);
    const host = await requireHost(options.host, options["js-engine"]);
    const start = () => {
        const child = spawn(host, hostArgs(base, { dir }), { stdio: "inherit" });
        child.on("exit", () => {});
        return child;
    };
    let child = start();
    console.log(`watching ${dir} for changes`);
    watch(dir, { recursive: true }, (_event, file) => {
        if (!file || file.includes("node_modules") || file.endsWith("~")) return;
        console.log(`reloading (${file})`);
        child.kill();
        child = start();
    });
}

function collectFiles(root, base = root, output = []) {
    for (const name of readdirSync(root)) {
        if (name.startsWith(".") || name === "node_modules" || name === "dist" || name === "target") continue;
        const full = join(root, name);
        const stats = statSync(full);
        if (stats.isDirectory()) {
            collectFiles(full, base, output);
        } else {
            output.push({ name: relative(base, full).split("\\").join("/"), path: full });
        }
    }
    return output;
}

async function bundle(entry, outfile) {
    let esbuild = null;
    try {
        esbuild = await import("esbuild");
    } catch {
        esbuild = null;
    }
    if (!esbuild) return false;
    await esbuild.build({
        entryPoints: [entry],
        bundle: true,
        format: "iife",
        outfile,
        logLevel: "warning",
    });
    return true;
}

async function createPak(dir, options) {
    const entryName = options.entry ?? "main.js";
    let bundled = false;
    if (options.bundle !== false) {
        const entry = join(dir, entryName);
        if (existsSync(entry)) {
            bundled = await bundle(entry, join(dir, ".kaplay-native-bundle.js"));
        }
    }
    const files = collectFiles(dir).filter((file) => !file.name.startsWith(".kaplay-native-bundle"));
    if (bundled) {
        files.push({ name: entryName, path: join(dir, ".kaplay-native-bundle.js") });
    }
    const entries = files.map((file) => ({ name: file.name, data: readFileSync(file.path) }));
    return { bytes: createZip(entries), count: entries.length, bundled };
}

async function buildCommand(options) {
    const dir = gameDir(options);
    const out = options.out ? resolve(options.out) : join(dir, "dist", `${basename(dir)}.pak`);
    mkdirSync(dirname(out), { recursive: true });
    const pak = await createPak(dir, options);
    writeFileSync(out, pak.bytes);
    console.log(`packed ${pak.count} files into ${out}${pak.bundled ? " (bundled with esbuild)" : ""}`);
}

function adhocSign(path, hostBytes) {
    if (!isMachO(hostBytes)) return;
    const result = spawnSync("codesign", ["--force", "--sign", "-", path], { encoding: "utf8" });
    if (result.error) {
        console.warn(`codesign is unavailable; sign ${path} manually before shipping it`);
    } else if (result.status !== 0) {
        console.warn(`codesign failed: ${(result.stderr || "").trim()}`);
    }
}

async function packCommand(options) {
    let dir = null;
    let pakBytes = null;
    let count = 0;
    let bundled = false;
    let name = "game";
    if (options.pak) {
        const source = resolve(options.pak);
        if (!existsSync(source)) {
            console.error(`pak archive not found: ${source}`);
            process.exit(1);
        }
        pakBytes = readFileSync(source);
        name = basename(source).replace(/\.pak$/i, "");
    } else {
        dir = gameDir(options);
        name = basename(dir);
        ({ bytes: pakBytes, count, bundled } = await createPak(dir, options));
    }
    const host = await requireHost(options.host, options["js-engine"]);
    if (options["js-engine"]) {
        const engines = hostEngines(host);
        if (engines && !engines.includes(options["js-engine"])) {
            console.error(
                `${host} has no ${options["js-engine"]} engine (available: ${engines.join(", ")})`,
            );
            console.error(`pass --host <path> to a host built with --features ${options["js-engine"]}`);
            process.exit(1);
        }
    }
    const tag = platformTag() ?? "unknown";
    const base = options.out ? resolve(options.out) : join(dir ?? process.cwd(), "dist", `${name}-${tag}`);
    const out = process.platform === "win32" && !base.endsWith(".exe") ? `${base}.exe` : base;
    mkdirSync(dirname(out), { recursive: true });
    const gameId = options["game-id"]
        ? requireGameId(options["game-id"])
        : options.title
          ? slugify(options.title) || null
          : null;
    const contentVersion = options["content-version"]
        ? requireSemver(options["content-version"], "--content-version")
        : null;
    const updateUrl = options["update-url"]
        ? requireHttpUrl(options["update-url"], "--update-url")
        : null;
    const updateKeys = options["update-key"]
        ? (Array.isArray(options["update-key"])
              ? options["update-key"]
              : [options["update-key"]]
          ).map(readPublicKey)
        : [];
    if (Boolean(updateUrl) !== (updateKeys.length > 0)) {
        fail("--update-url and --update-key must be used together");
    }
    if (updateUrl && (!gameId || !contentVersion)) {
        fail("--update-url also needs --game-id and --content-version");
    }
    const icon = options.icon ? loadIcon(requireValue(options, "icon", "--icon value")) : null;
    const config = {};
    if (options["js-engine"]) config.jsEngine = options["js-engine"];
    if (options.title) config.title = options.title;
    if (options.window) config.window = options.window;
    if (options["allow-network"]) config.network = true;
    if (options.url) config.url = options.url;
    if (gameId) config.gameId = gameId;
    if (contentVersion) config.contentVersion = contentVersion;
    if (icon) config.icon = icon.windowPng.toString("base64");
    if (updateUrl) {
        config.update =
            updateKeys.length === 1
                ? { url: updateUrl, publicKey: updateKeys[0] }
                : {
                      url: updateUrl,
                      publicKey: updateKeys[0],
                      publicKeys: updateKeys,
                  };
    }
    const configBytes = Buffer.from(JSON.stringify(config), "utf8");
    const trailer = Buffer.alloc(24);
    trailer.write(TRAILER_MAGIC, 0, "ascii");
    trailer.writeUInt32LE(TRAILER_VERSION, 8);
    trailer.writeBigUInt64LE(BigInt(pakBytes.length), 12);
    trailer.writeUInt32LE(configBytes.length, 20);
    let hostBytes = readFileSync(host);
    if (icon) {
        if (isPe(hostBytes)) {
            try {
                hostBytes = patchWindowsIcon(hostBytes, icon.ico);
            } catch (error) {
                fail(`failed to add the Windows icon: ${error.message}`);
            }
        } else if (isMachO(hostBytes)) {
            writeFileSync(`${out}.icns`, icon.icns);
        }
    }
    writeFileSync(out, Buffer.concat([hostBytes, pakBytes, configBytes, trailer]));
    if (process.platform !== "win32") chmodSync(out, 0o755);
    adhocSign(out, hostBytes);
    const summary = options.pak ? `embedded ${basename(options.pak)}` : `packed ${count} files`;
    console.log(`${summary} into ${out}${bundled ? " (bundled with esbuild)" : ""}`);
    console.log(`run it with: ${out}`);
}

async function testCommand(options) {
    const dir = gameDir(options);
    const screenshot = resolve(options.screenshot ?? join(dir, "test", "actual.png"));
    const golden = resolve(options.golden ?? join(dir, "test", "golden.png"));
    mkdirSync(dirname(screenshot), { recursive: true });
    const args = hostArgs({ ...options, frames: options.frames ?? 120, screenshot, headless: true }, { dir });
    const result = await withHost(options, args, { capture: true });
    if (result.status !== 0) {
        console.error(result.stdout);
        console.error(result.stderr);
        process.exit(result.status ?? 1);
    }
    if (options.update) {
        copyFileSync(screenshot, golden);
        console.log(`updated golden screenshot ${golden}`);
        return;
    }
    if (!existsSync(golden)) {
        console.log(`no golden screenshot at ${golden}; wrote ${screenshot} (run with --update to accept)`);
        return;
    }
    const actual = readFileSync(screenshot);
    const expected = readFileSync(golden);
    if (actual.equals(expected)) {
        console.log("screenshot matches golden");
        return;
    }
    console.error(`screenshot differs from golden (${actual.length} vs ${expected.length} bytes)`);
    console.error(`actual: ${screenshot}`);
    process.exit(1);
}

async function doctorCommand(options) {
    const checks = [];
    checks.push(["node", process.version]);
    checks.push(["platform", `${process.platform} ${process.arch}`]);
    checks.push(["host release", hostAssetName() ?? "unsupported"]);
    checks.push(["host v8 release", hostAssetName("v8") ?? "unsupported"]);
    checks.push(["host cache", cacheDir()]);
    const host = await findHost(options.host, options["js-engine"]);
    checks.push(["host binary", host ?? "not found"]);
    checks.push(["DISPLAY", process.env.DISPLAY ?? "(unset)"]);
    checks.push(["WAYLAND_DISPLAY", process.env.WAYLAND_DISPLAY ?? "(unset)"]);
    if (host) {
        const version = spawnSync(host, ["--version"], { encoding: "utf8" });
        checks.push(["host version", (version.stdout || version.stderr || "").trim() || "unknown"]);
    }
    for (const [name, value] of checks) {
        console.log(`${name.padEnd(18)} ${value}`);
    }
}

function keygenCommand(options) {
    const out = options.out ? resolve(options.out) : resolve("update-key.pem");
    if (existsSync(out) && !options.force) {
        fail(`${out} already exists; pass --force to overwrite it`);
    }
    mkdirSync(dirname(out), { recursive: true });
    const { publicKey, privateKey } = generateKeyPairSync("ed25519");
    const privatePem = privateKey.export({ type: "pkcs8", format: "pem" });
    const spki = publicKey.export({ type: "spki", format: "der" });
    const raw = spki.subarray(spki.length - 32);
    const publicText = raw.toString("base64");
    const pubPath = `${out}.pub`;
    writeFileSync(out, privatePem, { mode: 0o600 });
    writeFileSync(pubPath, `${publicText}\n`);
    console.log(`private key ${out} (keep it secret)`);
    console.log(`public key  ${pubPath} ${publicText}`);
}

function manifestCommand(options) {
    const pakPath = options._[0] ? resolve(options._[0]) : null;
    if (!pakPath || !existsSync(pakPath) || !statSync(pakPath).isFile()) {
        fail(`pak archive not found: ${pakPath ?? "(missing)"}`);
    }
    const version = requireSemver(requireValue(options, "version", "--version"), "--version");
    const url = requireHttpUrl(requireValue(options, "url", "--url"), "--url");
    const gameId = requireGameId(requireValue(options, "game-id", "--game-id"));
    const keyPath = resolve(requireValue(options, "key", "--key"));
    if (!existsSync(keyPath) || !statSync(keyPath).isFile()) fail(`private key not found: ${keyPath}`);
    const minHost = options["min-host"]
        ? requireSemver(options["min-host"], "--min-host")
        : null;
    const rollout = options.rollout !== undefined ? requireRollout(options.rollout) : null;
    const pak = readFileSync(pakPath);
    if (pak.length === 0) fail(`pak archive is empty: ${pakPath}`);
    const sha256 = createHash("sha256").update(pak).digest("hex");
    const manifest = {
        format: 1,
        gameId,
        content: {
            version,
            ...(minHost ? { minHost } : {}),
            url,
            sha256,
            size: pak.length,
            ...(rollout !== null ? { rollout } : {}),
            ...(options.required ? { required: true } : {}),
        },
    };
    const bytes = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`, "utf8");
    let privateKey;
    try {
        privateKey = createPrivateKey(readFileSync(keyPath));
    } catch (error) {
        fail(`could not read private key ${keyPath}: ${error.message}`);
    }
    if (privateKey.asymmetricKeyType !== "ed25519") {
        fail(`private key ${keyPath} is not an Ed25519 key`);
    }
    const signature = sign(null, bytes, privateKey);
    const target = options.out ? resolve(options.out) : dirname(pakPath);
    const manifestPath = target.endsWith(".json") ? target : join(target, "manifest.json");
    mkdirSync(dirname(manifestPath), { recursive: true });
    writeFileSync(manifestPath, bytes);
    writeFileSync(`${manifestPath}.sig`, `${signature.toString("base64")}\n`);
    console.log(`content ${version} sha256 ${sha256} (${pak.length} bytes)`);
    console.log(`wrote ${manifestPath} and ${manifestPath}.sig`);
}

async function enginesCommand(options) {
    const host = await requireHost(options.host, options["js-engine"]);
    const result = spawnSync(host, ["--engines"], { encoding: "utf8" });
    if (result.status !== 0) {
        console.error(result.stderr || "failed to list javascript engines");
        process.exit(result.status ?? 1);
    }
    process.stdout.write(result.stdout);
}

const command = process.argv[2];
const options = parseArgs(process.argv.slice(3));

if (!command || options.help) {
    usage();
    process.exit(command ? 0 : 1);
}

switch (command) {
    case "run":
        await runCommand(options);
        break;
    case "dev":
        await devCommand(options);
        break;
    case "build":
        await buildCommand(options);
        break;
    case "pack":
        await packCommand(options);
        break;
    case "test":
        await testCommand(options);
        break;
    case "keygen":
        keygenCommand(options);
        break;
    case "manifest":
        manifestCommand(options);
        break;
    case "engines":
        await enginesCommand(options);
        break;
    case "doctor":
        await doctorCommand(options);
        break;
    default:
        console.error(`unknown command: ${command}`);
        usage();
        process.exit(1);
}
