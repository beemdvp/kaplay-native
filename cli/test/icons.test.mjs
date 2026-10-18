import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { PNG } from "pngjs";

import { isMachO, isPe, prepareIcon } from "../src/icon.mjs";

const cli = fileURLToPath(new URL("../src/index.mjs", import.meta.url));

function run(args, cwd) {
    return spawnSync(process.execPath, [cli, ...args], { cwd, encoding: "utf8" });
}

function workspace(t) {
    const dir = mkdtempSync(join(tmpdir(), "kaplay-native-icons-"));
    t.after(() => rmSync(dir, { recursive: true, force: true }));
    return dir;
}

function squarePng(size) {
    const png = new PNG({ width: size, height: size });
    for (let pixel = 0; pixel < size * size; pixel++) {
        const at = pixel * 4;
        png.data[at] = 200;
        png.data[at + 1] = 60;
        png.data[at + 2] = 30;
        png.data[at + 3] = 255;
    }
    return PNG.sync.write(png);
}

function readConfig(path) {
    const bytes = readFileSync(path);
    const trailer = bytes.subarray(bytes.length - 24);
    assert.equal(trailer.subarray(0, 8).toString("ascii"), "KNATIVEP");
    const configLength = trailer.readUInt32LE(20);
    return JSON.parse(bytes.subarray(bytes.length - 24 - configLength, bytes.length - 24).toString("utf8"));
}

function icnsChunks(bytes) {
    const chunks = [];
    let at = 8;
    while (at < bytes.length) {
        chunks.push({ type: bytes.subarray(at, at + 4).toString("ascii"), length: bytes.readUInt32BE(at + 4) });
        at += bytes.readUInt32BE(at + 4);
    }
    assert.equal(at, bytes.length);
    assert.equal(bytes.readUInt32BE(4), bytes.length);
    return chunks;
}

test("prepareIcon builds a window png, an ico, and an icns", () => {
    const icon = prepareIcon(squarePng(512));
    const window = PNG.sync.read(icon.windowPng);
    assert.equal(window.width, 128);
    assert.equal(window.height, 128);

    const count = icon.ico.readUInt16LE(4);
    assert.equal(count, 6);
    assert.equal(icon.ico.readUInt16LE(2), 1);
    const sizes = [];
    for (let index = 0; index < count; index++) {
        const at = 6 + index * 16;
        const size = icon.ico[at] || 256;
        sizes.push(size);
        const length = icon.ico.readUInt32LE(at + 8);
        const offset = icon.ico.readUInt32LE(at + 12);
        assert.ok(offset + length <= icon.ico.length);
        assert.equal(icon.ico.subarray(offset, offset + 4).toString("hex"), "89504e47");
        assert.equal(PNG.sync.read(icon.ico.subarray(offset, offset + length)).width, size);
    }
    assert.deepEqual(sizes, [16, 32, 48, 64, 128, 256]);

    const chunks = icnsChunks(icon.icns);
    const types = chunks.map((chunk) => chunk.type);
    assert.deepEqual(types, [
        "icp4",
        "icp5",
        "icp6",
        "ic07",
        "ic08",
        "ic09",
        "ic11",
        "ic12",
        "ic13",
        "ic14",
    ]);
});

test("prepareIcon rejects icons that cannot be resized", () => {
    assert.throws(() => prepareIcon(Buffer.from("not a png")), /PNG/);
    const wide = new PNG({ width: 512, height: 256 });
    assert.throws(() => prepareIcon(PNG.sync.write(wide)), /square/);
    assert.throws(() => prepareIcon(squarePng(128)), /256 to 2048/);
});

test("platform detection finds PE and Mach-O hosts", () => {
    assert.equal(isPe(Buffer.from("MZ")), true);
    assert.equal(isPe(Buffer.from("host bytes")), false);
    assert.equal(isMachO(Buffer.from([0xcf, 0xfa, 0xed, 0xfe])), true);
    assert.equal(isMachO(Buffer.from("host bytes")), false);
    assert.equal(isMachO(Buffer.from([0xca, 0xfe, 0xba, 0xbe])), true);
});

test("pack embeds the window icon in the config", (t) => {
    const dir = workspace(t);
    writeFileSync(join(dir, "game.pak"), "pak contents");
    writeFileSync(join(dir, "host"), "host bytes");
    writeFileSync(join(dir, "icon.png"), squarePng(512));
    const output = process.platform === "win32" ? "packed.exe" : "packed";
    const result = run(["pack", "--pak", "game.pak", "--host", "host", "--out", "packed", "--icon", "icon.png"], dir);
    assert.equal(result.status, 0, result.stderr);
    const config = readConfig(join(dir, output));
    assert.equal(typeof config.icon, "string");
    const window = PNG.sync.read(Buffer.from(config.icon, "base64"));
    assert.equal(window.width, 128);
    assert.equal(window.height, 128);
    assert.equal(existsSync(join(dir, `${output}.icns`)), false);
});

test("pack writes an icns beside a Mach-O host", (t) => {
    const dir = workspace(t);
    writeFileSync(join(dir, "game.pak"), "pak contents");
    writeFileSync(join(dir, "host"), Buffer.concat([Buffer.from([0xcf, 0xfa, 0xed, 0xfe]), Buffer.alloc(64)]));
    writeFileSync(join(dir, "icon.png"), squarePng(512));
    const output = process.platform === "win32" ? "packed.exe" : "packed";
    const result = run(["pack", "--pak", "game.pak", "--host", "host", "--out", "packed", "--icon", "icon.png"], dir);
    assert.equal(result.status, 0, result.stderr);
    const icns = join(dir, `${output}.icns`);
    assert.ok(existsSync(icns));
    assert.equal(readFileSync(icns).subarray(0, 4).toString("ascii"), "icns");
    assert.equal(typeof readConfig(join(dir, output)).icon, "string");
});
