import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash, createPublicKey, verify } from "node:crypto";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const cli = fileURLToPath(new URL("../src/index.mjs", import.meta.url));
const SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");

function run(args, cwd) {
    return spawnSync(process.execPath, [cli, ...args], { cwd, encoding: "utf8" });
}

function workspace(t) {
    const dir = mkdtempSync(join(tmpdir(), "kaplay-native-updates-"));
    t.after(() => rmSync(dir, { recursive: true, force: true }));
    return dir;
}

function publicKeyFromFile(path) {
    const raw = Buffer.from(readFileSync(path, "utf8").trim(), "base64");
    return createPublicKey({
        key: Buffer.concat([SPKI_PREFIX, raw]),
        format: "der",
        type: "spki",
    });
}

function readTrailerConfig(path) {
    const bytes = readFileSync(path);
    const trailer = bytes.subarray(bytes.length - 24);
    assert.equal(trailer.subarray(0, 8).toString("ascii"), "KNATIVEP");
    assert.equal(trailer.readUInt32LE(8), 1);
    const pakLength = Number(trailer.readBigUInt64LE(12));
    const configLength = trailer.readUInt32LE(20);
    const configStart = bytes.length - 24 - configLength;
    const pakStart = configStart - pakLength;
    assert.ok(pakStart >= 0);
    assert.equal(bytes.subarray(pakStart, configStart).toString("utf8"), "pak contents");
    return JSON.parse(bytes.subarray(configStart, bytes.length - 24).toString("utf8"));
}

test("keygen writes a private key and a raw public key", (t) => {
    const dir = workspace(t);
    const result = run(["keygen", "--out", "signing.pem"], dir);
    assert.equal(result.status, 0, result.stderr);
    assert.ok(existsSync(join(dir, "signing.pem")));
    const publicKey = readFileSync(join(dir, "signing.pem.pub"), "utf8").trim();
    assert.equal(Buffer.from(publicKey, "base64").length, 32);
});

test("manifest signs the exact manifest bytes", (t) => {
    const dir = workspace(t);
    const pak = Buffer.from("pak contents");
    writeFileSync(join(dir, "game.pak"), pak);
    assert.equal(run(["keygen", "--out", "signing.pem"], dir).status, 0);

    const result = run(
        [
            "manifest",
            "game.pak",
            "--version",
            "1.2.0",
            "--url",
            "https://cdn.example.com/game.pak",
            "--game-id",
            "bridge",
            "--key",
            "signing.pem",
            "--min-host",
            "0.1.0",
            "--out",
            "dist",
        ],
        dir,
    );
    assert.equal(result.status, 0, result.stderr);
    const bytes = readFileSync(join(dir, "dist", "manifest.json"));
    const manifest = JSON.parse(bytes.toString("utf8"));
    assert.equal(manifest.format, 1);
    assert.equal(manifest.gameId, "bridge");
    assert.equal(manifest.content.version, "1.2.0");
    assert.equal(manifest.content.minHost, "0.1.0");
    assert.equal(manifest.content.sha256, createHash("sha256").update(pak).digest("hex"));
    assert.equal(manifest.content.size, pak.length);
    const signature = Buffer.from(
        readFileSync(join(dir, "dist", "manifest.json.sig"), "utf8").trim(),
        "base64",
    );
    const publicKey = publicKeyFromFile(join(dir, "signing.pem.pub"));
    assert.ok(verify(null, bytes, publicKey, signature));
});

test("pack embeds the update settings", (t) => {
    const dir = workspace(t);
    writeFileSync(join(dir, "game.pak"), "pak contents");
    writeFileSync(join(dir, "host"), "host bytes");
    assert.equal(run(["keygen", "--out", "signing.pem"], dir).status, 0);
    const output = process.platform === "win32" ? "packed.exe" : "packed";
    const result = run(
        [
            "pack",
            "--pak",
            "game.pak",
            "--host",
            "host",
            "--out",
            "packed",
            "--game-id",
            "bridge",
            "--content-version",
            "1.2.0",
            "--update-url",
            "https://cdn.example.com/stable/manifest.json",
            "--update-key",
            "signing.pem.pub",
        ],
        dir,
    );
    assert.equal(result.status, 0, result.stderr);
    const config = readTrailerConfig(join(dir, output));
    assert.equal(config.gameId, "bridge");
    assert.equal(config.contentVersion, "1.2.0");
    assert.equal(config.update.url, "https://cdn.example.com/stable/manifest.json");
    assert.equal(
        config.update.publicKey,
        readFileSync(join(dir, "signing.pem.pub"), "utf8").trim(),
    );
});

test("pack rejects an incomplete update configuration", (t) => {
    const dir = workspace(t);
    writeFileSync(join(dir, "game.pak"), "pak contents");
    writeFileSync(join(dir, "host"), "host bytes");
    const result = run(
        [
            "pack",
            "--pak",
            "game.pak",
            "--host",
            "host",
            "--out",
            "packed",
            "--update-url",
            "https://cdn.example.com/manifest.json",
        ],
        dir,
    );
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /--update-key/);
});

test("manifest rejects a non-semver version", (t) => {
    const dir = workspace(t);
    writeFileSync(join(dir, "game.pak"), "pak contents");
    assert.equal(run(["keygen", "--out", "signing.pem"], dir).status, 0);
    const result = run(
        [
            "manifest",
            "game.pak",
            "--version",
            "latest",
            "--url",
            "https://cdn.example.com/game.pak",
            "--game-id",
            "bridge",
            "--key",
            "signing.pem",
        ],
        dir,
    );
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /semver/);
});
