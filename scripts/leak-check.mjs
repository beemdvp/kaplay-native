import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
let host = resolve(repo, "target", "release", "kaplay-native-host");
let verbose = false;
for (let index = 2; index < process.argv.length; index++) {
    const arg = process.argv[index];
    if (arg === "--host") {
        host = resolve(process.argv[++index]);
    } else if (arg === "--verbose") {
        verbose = true;
    } else {
        console.error(`unknown option: ${arg}`);
        process.exit(2);
    }
}

const scenarios = [
    {
        name: "images",
        light: 10,
        heavy: 200,
        frames: 20,
        source: (count) => `
const source = document.createElement("canvas");
source.width = 512;
source.height = 512;
const context = source.getContext("2d");
context.fillStyle = "#fff";
context.fillRect(0, 0, 512, 512);
const url = source.toDataURL();
for (let i = 0; i < ${count}; i++) {
    const image = new Image();
    image.src = url;
}
let junk = [];
for (let i = 0; i < 300; i++) junk.push(new Uint8Array(1024 * 1024));
junk = null;
console.log("LEAK_SETUP images");
`,
        metric: (report) => report.images,
        limit: 0,
    },
    {
        name: "image-src",
        light: 10,
        heavy: 200,
        frames: 20,
        source: (count) => `
const source = document.createElement("canvas");
source.width = 64;
source.height = 64;
const context = source.getContext("2d");
context.fillStyle = "#fff";
context.fillRect(0, 0, 64, 64);
const url = source.toDataURL();
const image = new Image();
for (let i = 0; i < ${count}; i++) image.src = url;
image.__release();
console.log("LEAK_SETUP image-src");
`,
        metric: (report) => report.images,
        limit: 0,
    },
    {
        name: "canvases",
        light: 10,
        heavy: 200,
        frames: 20,
        source: (count) => `
const pixels = new Uint8ClampedArray(512 * 512 * 4);
pixels.fill(200);
for (let i = 0; i < ${count}; i++) {
    const canvas = document.createElement("canvas");
    canvas.width = 512;
    canvas.height = 512;
    document.body.appendChild(canvas);
    canvas.getContext("2d").putImageData(new ImageData(pixels, 512, 512), 0, 0);
    document.body.removeChild(canvas);
}
let junk = [];
for (let i = 0; i < 300; i++) junk.push(new Uint8Array(1024 * 1024));
junk = null;
console.log("LEAK_SETUP canvases");
`,
        metric: (report) => report.canvases + report.contexts,
        limit: 0,
    },
    {
        name: "uniforms",
        light: 1_000,
        heavy: 2_000_000,
        frames: 20,
        source: (count) => `
const canvas = document.createElement("canvas");
document.body.appendChild(canvas);
const gl = canvas.getContext("webgl");
if (!gl) throw new Error("no gl");
const vertex = gl.createShader(gl.VERTEX_SHADER);
gl.shaderSource(vertex, "attribute vec2 a_pos;void main(){gl_Position=vec4(a_pos,0.0,1.0);}");
gl.compileShader(vertex);
const fragment = gl.createShader(gl.FRAGMENT_SHADER);
gl.shaderSource(fragment, "precision mediump float;uniform float u_test;void main(){gl_FragColor=vec4(u_test);}");
gl.compileShader(fragment);
const program = gl.createProgram();
gl.attachShader(program, vertex);
gl.attachShader(program, fragment);
gl.linkProgram(program);
let last = -1;
for (let i = 0; i < ${count}; i++) last = gl.getUniformLocation(program, "u_test");
console.log("LEAK_SETUP uniforms", last);
`,
        metric: (report) => report.uniform_locations,
        limit: 0,
    },
    {
        name: "audio",
        light: 10,
        heavy: 300,
        frames: 120,
        source: (count) => `
function wav(frames, sampleRate) {
    const bytes = new Uint8Array(44 + frames * 2);
    const view = new DataView(bytes.buffer);
    const write = (offset, text) => { for (let i = 0; i < text.length; i++) bytes[offset + i] = text.charCodeAt(i); };
    write(0, "RIFF");
    view.setUint32(4, 36 + frames * 2, true);
    write(8, "WAVE");
    write(12, "fmt ");
    view.setUint32(16, 16, true);
    view.setUint16(20, 1, true);
    view.setUint16(22, 1, true);
    view.setUint32(24, sampleRate, true);
    view.setUint32(28, sampleRate * 2, true);
    view.setUint16(32, 2, true);
    view.setUint16(34, 16, true);
    write(36, "data");
    view.setUint32(40, frames * 2, true);
    return bytes;
}
const data = wav(48000, 48000);
const context = new AudioContext();
const pending = [];
for (let i = 0; i < ${count}; i++) pending.push(context.decodeAudioData(data.buffer.slice(0)));
Promise.all(pending).then(() => {
    pending.length = 0;
    let junk = [];
    for (let i = 0; i < 300; i++) junk.push(new Uint8Array(1024 * 1024));
    junk = null;
    console.log("LEAK_SETUP audio");
});
`,
        metric: (report) => report.audio_buffers,
        limit: 0,
    },
];

const root = mkdtempSync(join(tmpdir(), "kaplay-native-leak-"));

function run(script, scenario) {
    const report = join(root, `${scenario.name}-report.json`);
    rmSync(report, { force: true });
    const args = [
        host,
        "--script", script,
        "--headless",
        "--window", "320x240",
        "--frames", String(scenario.frames),
        "--no-steam",
        "--no-vsync",
        "--resource-report", report,
        "--no-audio",
    ];
    const result = spawnSync(args[0], args.slice(1), {
        encoding: "utf8",
        timeout: 120_000,
        maxBuffer: 16 * 1024 * 1024,
    });
    if (verbose && result.stderr) process.stderr.write(result.stderr);
    if (result.error) throw result.error;
    if (result.status !== 0) {
        throw new Error(`host exited with ${result.status}\n${result.stdout}\n${result.stderr}`);
    }
    return JSON.parse(readFileSync(report, "utf8"));
}

let failed = false;
for (const scenario of scenarios) {
    const lightPath = join(root, `${scenario.name}-light.js`);
    const heavyPath = join(root, `${scenario.name}-heavy.js`);
    writeFileSync(lightPath, scenario.source(scenario.light));
    writeFileSync(heavyPath, scenario.source(scenario.heavy));
    let light;
    let heavy;
    try {
        light = scenario.metric(run(lightPath, scenario));
        heavy = scenario.metric(run(heavyPath, scenario));
    } catch (error) {
        failed = true;
        console.error(`FAIL ${scenario.name}: ${error instanceof Error ? error.message : error}`);
        continue;
    }
    const delta = heavy - light;
    const ok = delta <= scenario.limit;
    if (!ok) failed = true;
    console.log(
        `${ok ? "PASS" : "FAIL"} ${scenario.name}: light ${light}, heavy ${heavy}, ` +
        `delta ${delta} (limit ${scenario.limit})`,
    );
}

rmSync(root, { recursive: true, force: true });
if (failed) process.exit(1);
