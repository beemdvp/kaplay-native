import { spawnSync } from "node:child_process";
import {
    existsSync,
    mkdirSync,
    readFileSync,
    rmSync,
    writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "..");
const platform = `${process.platform}-${process.arch}`;
const warnThreshold = 0.05;
const failThreshold = 0.1;
const minGatedValue = 20_000;
const measureMargin = 20;

const scenarios = {
    idle: { count: 0, frames: 320 },
    draw: { count: 400, frames: 320 },
    move: { count: 400, frames: 320 },
    text: { count: 400, frames: 320 },
    collide: { count: 200, frames: 16 },
    churn: { count: 400, frames: 320 },
    events: { count: 400, frames: 320 },
    microquad: { count: 400, frames: 320 },
    microsprite: { count: 400, frames: 320 },
    micromatrix: { count: 400, frames: 320 },
};

function usage() {
    console.log(`kaplay js bench

  node scripts/bench-kaplay.mjs run       [options]   run and write benchmarks/results
  node scripts/bench-kaplay.mjs baseline  [options]   run and replace the committed baseline
  node scripts/bench-kaplay.mjs compare   [options]   run and diff against the baseline

Options:
  --engine <name>     quickjs (default), v8, or both
  --kaplay <path>     kaplay bundle to preload (default ../kaplay/dist/kaplay.js)
  --scenario <name>   limit to one scenario: ${Object.keys(scenarios).join(", ")}
  --frames <n>        host frames for every scenario
  --count <n>         object count for every scenario
  --host <path>       host binary for the quickjs engine
  --v8-host <path>    host binary for the v8 engine
  --repeats <n>       runs per scenario, best JS p50 wins (default 2)
  --no-build          reuse target/release/kaplay-native-host-<engine>
  --verbose           print host output
`);
}

function parse(argv) {
    const options = {
        mode: argv[0] ?? "run",
        engine: "quickjs",
        repeats: 2,
        build: true,
    };
    for (let index = 1; index < argv.length; index++) {
        const arg = argv[index];
        const next = argv[index + 1];
        switch (arg) {
            case "--engine":
                options.engine = next;
                index++;
                break;
            case "--kaplay":
                options.kaplay = next;
                index++;
                break;
            case "--scenario":
                options.scenario = next;
                index++;
                break;
            case "--frames":
                options.frames = Number(next);
                index++;
                break;
            case "--count":
                options.count = Number(next);
                index++;
                break;
            case "--host":
                options.host = next;
                index++;
                break;
            case "--v8-host":
                options.v8Host = next;
                index++;
                break;
            case "--repeats":
                options.repeats = Math.max(1, Number(next));
                index++;
                break;
            case "--no-build":
                options.build = false;
                break;
            case "--verbose":
                options.verbose = true;
                break;
            case "--help":
                usage();
                process.exit(0);
                break;
            default:
                console.error(`unknown option: ${arg}`);
                process.exit(2);
        }
    }
    return options;
}

function run(command, args, options = {}) {
    return spawnSync(command, args, {
        cwd: options.cwd ?? repo,
        encoding: "utf8",
        timeout: options.timeout ?? 1_800_000,
        maxBuffer: 64 * 1024 * 1024,
    });
}

function hostBinaryPath(engine) {
    return join(repo, "target", "release", `kaplay-native-host-${engine}`);
}

function buildHosts(options) {
    const engines = options.engine === "both" ? ["quickjs", "v8"] : [options.engine];
    const binaries = {};
    for (const engine of engines) {
        const explicit = engine === "v8" ? options.v8Host : options.host;
        if (explicit) {
            binaries[engine] = resolve(explicit);
            continue;
        }
        const cached = hostBinaryPath(engine);
        if (!options.build && existsSync(cached)) {
            binaries[engine] = cached;
            continue;
        }
        const args = ["build", "--release", "-p", "kaplay-native-host"];
        if (engine === "v8") args.push("--no-default-features", "--features", "v8");
        console.log(`building the ${engine} host`);
        const result = run("cargo", args);
        if (result.status !== 0) {
            console.error(result.stderr || result.stdout);
            throw new Error(`failed to build the ${engine} host`);
        }
        const built = join(repo, "target", "release", "kaplay-native-host");
        const copied = run("cp", [built, cached]);
        if (copied.status !== 0) throw new Error(`failed to copy the ${engine} host`);
        binaries[engine] = cached;
    }
    return binaries;
}

function kaplayInfo(path) {
    const directory = dirname(path);
    const git = run("git", ["-C", directory, "rev-parse", "HEAD"], { timeout: 30_000 });
    const head = git.status === 0 ? git.stdout.trim().split("\n")[0] : "";
    return {
        path: resolve(path),
        bytes: existsSync(path) ? readFileSync(path).length : 0,
        sha256: existsSync(path)
            ? createHash("sha256").update(readFileSync(path)).digest("hex").slice(0, 16)
            : "",
        commit: head,
    };
}

function benchLines(output, marker) {
    const values = {};
    const pattern = new RegExp(`${marker} ([^=]+)=(.+?)\\s*$`, "gm");
    for (const match of output.matchAll(pattern)) values[match[1]] = match[2];
    return values;
}

function runScenario(binary, kaplayPath, name, config, options) {
    const hostFrames = options.frames ?? config.frames;
    const count = options.count ?? config.count;
    const measureFrames = Math.max(5, hostFrames - measureMargin);
    const jsonPath = join(
        tmpdir(),
        `kaplay-js-bench-${process.pid}-${name}.json`,
    );
    const url = `http://127.0.0.1:1/index.html?scenario=${name}&count=${count}&frames=${measureFrames}`;
    const result = run(binary, [
        "--script",
        "benchmarks/kaplay/main.js",
        "--dir",
        "benchmarks/kaplay",
        "--preload",
        kaplayPath,
        "--window",
        "960x540",
        "--frames",
        String(hostFrames),
        "--no-vsync",
        "--no-audio",
        "--headless",
        "--x11",
        "--url",
        url,
        "--bench-json",
        jsonPath,
    ]);
    let bench = null;
    if (existsSync(jsonPath)) {
        try {
            bench = JSON.parse(readFileSync(jsonPath, "utf8"));
        } catch (error) {
            bench = { error: String(error) };
        }
        rmSync(jsonPath, { force: true });
    }
    const output = `${result.stdout}\n${result.stderr}`;
    if (options.verbose) console.log(output.trim());
    return {
        status: result.status,
        renderer: /renderer: (.+)/.exec(output)?.[1]?.trim() ?? "",
        checks: benchLines(output, "BENCH"),
        info: benchLines(output, "BENCH_INFO"),
        bench,
    };
}

function scenarioScore(scenario) {
    const p50 = scenario.bench?.phases_ms?.js?.p50;
    return typeof p50 === "number" ? p50 : Infinity;
}

function gather(options, binaries, kaplayPath) {
    const names = options.scenario ? [options.scenario] : Object.keys(scenarios);
    for (const name of names) {
        if (!scenarios[name]) throw new Error(`unknown scenario: ${name}`);
    }
    const result = {
        created: new Date().toISOString(),
        platform,
        kaplay: kaplayInfo(kaplayPath),
        engines: {},
    };
    for (const [engine, binary] of Object.entries(binaries)) {
        const entry = { scenarios: {}, renderer: "" };
        for (const name of names) {
            const runs = [];
            for (let repeat = 0; repeat < options.repeats; repeat++) {
                console.log(
                    `kaplay ${name} (${engine}, ${scenarios[name].count} objects, ${repeat + 1}/${options.repeats})`,
                );
                runs.push(
                    runScenario(binary, kaplayPath, name, scenarios[name], options),
                );
            }
            for (const key of Object.keys(runs[0].checks)) {
                for (const run of runs.slice(1)) {
                    if (run.checks[key] !== runs[0].checks[key]) {
                        console.error(
                            `nondeterministic check ${name}.${key}: ${runs[0].checks[key]} vs ${run.checks[key]}`,
                        );
                        process.exitCode = 1;
                    }
                }
            }
            const scenario = runs.reduce((best, run) =>
                scenarioScore(run) < scenarioScore(best) ? run : best
            );
            scenario.runs = runs.map((run) => ({
                status: run.status,
                js_p50: Number.isFinite(scenarioScore(run))
                    ? scenarioScore(run)
                    : null,
            }));
            entry.scenarios[name] = scenario;
            if (scenario.renderer) entry.renderer = scenario.renderer;
        }
        result.engines[engine] = entry;
    }
    return result;
}

function flatten(value, prefix, out) {
    for (const [key, entry] of Object.entries(value)) {
        const path = prefix ? `${prefix}.${key}` : key;
        if (typeof entry === "number") {
            out[path] = entry;
        } else if (entry && typeof entry === "object") {
            flatten(entry, path, out);
        }
    }
    return out;
}

function timingMetrics(entry) {
    const metrics = {};
    for (const [name, scenario] of Object.entries(entry.scenarios)) {
        if (name === "idle" || name === "micromatrix") continue;
        if (scenario.bench && typeof scenario.bench === "object") {
            const flat = flatten(scenario.bench, `${name}.bench`, {});
            for (const [key, value] of Object.entries(flat)) {
                const relevant = (key.endsWith(".phases_ms.js.p50")
                    || key.endsWith(".fps"))
                    && !key.includes(".micromatrix.");
                if (!relevant) continue;
                metrics[key] = key.endsWith(".fps") ? value : value * 1e6;
            }
        }
        for (const [key, value] of Object.entries(scenario.info)) {
            if (!key.includes("loop_ms") || key.includes("micromatrix")) continue;
            const parsed = Number(value);
            if (Number.isFinite(parsed)) {
                metrics[`${name}.info.${key}`] = parsed * 1e6;
            }
        }
    }
    return metrics;
}

function better(key) {
    return key.endsWith(".fps") ? "up" : "down";
}

function diff(baseline, current) {
    const rows = [];
    let failures = 0;
    let warnings = 0;
    for (const [key, value] of Object.entries(current)) {
        const previous = baseline[key];
        if (!Number.isFinite(value) || !Number.isFinite(previous) || previous === 0) {
            rows.push({ key, delta: null });
            continue;
        }
        const raw = (value - previous) / previous;
        const delta = better(key) === "up" ? -raw : raw;
        const gated = !key.endsWith(".fps") && previous >= minGatedValue;
        rows.push({ key, delta, previous, value, gated });
        if (!gated) continue;
        if (delta > failThreshold) failures++;
        else if (delta > warnThreshold) warnings++;
    }
    rows.sort((a, b) => {
        if (a.delta === null && b.delta === null) return 0;
        if (a.delta === null) return -1;
        if (b.delta === null) return 1;
        return b.delta - a.delta;
    });
    return { rows, failures, warnings };
}

function report(baseline, current) {
    const { rows, failures, warnings } = diff(baseline, current);
    console.log(`\nbiggest regressions:`);
    for (const row of rows.slice(0, 15)) {
        const tag = row.delta === null
            ? "new  "
            : `${(row.delta * 100).toFixed(1)}%`.padStart(6);
        console.log(`  ${tag}${row.gated === false ? " *" : "  "}  ${row.key}`);
    }
    console.log(`\nbiggest improvements:`);
    for (const row of rows.slice(-8).reverse()) {
        if (row.delta === null) continue;
        console.log(`  ${`${(row.delta * 100).toFixed(1)}%`.padStart(8)}  ${row.key}`);
    }
    console.log(`\n${failures} failing, ${warnings} warning (* below the ${minGatedValue} ns gate)`);
    return failures;
}

function checkChecks(baseline, current, label) {
    let failures = 0;
    const engines = Object.keys(current.engines);
    for (const engine of engines) {
        const baseChecks = baseline.engines[engine]?.scenarios ?? {};
        const now = current.engines[engine].scenarios;
        for (const name of new Set([...Object.keys(baseChecks), ...Object.keys(now)])) {
            const left = baseChecks[name]?.checks ?? {};
            const right = now[name]?.checks ?? {};
            for (const key of new Set([...Object.keys(left), ...Object.keys(right)])) {
                if (left[key] !== right[key]) {
                    console.error(
                        `${label} mismatch ${engine}.${name}.${key}: ${left[key]} -> ${right[key]}`,
                    );
                    failures++;
                }
            }
        }
    }
    if (engines.length > 1) {
        const [first, second] = engines;
        for (const name of Object.keys(current.engines[first].scenarios)) {
            const left = current.engines[first].scenarios[name]?.checks ?? {};
            const right = current.engines[second].scenarios[name]?.checks ?? {};
            for (const key of new Set([...Object.keys(left), ...Object.keys(right)])) {
                if (left[key] !== right[key]) {
                    console.error(
                        `parity mismatch ${name}.${key}: ${first}=${left[key]} ${second}=${right[key]}`,
                    );
                    failures++;
                }
            }
        }
    }
    return failures;
}

function main() {
    const options = parse(process.argv.slice(2));
    if (!["run", "baseline", "compare"].includes(options.mode)) {
        usage();
        process.exit(2);
    }
    const kaplayPath = resolve(
        options.kaplay ?? join(repo, "..", "kaplay", "dist", "kaplay.js"),
    );
    if (!existsSync(kaplayPath)) {
        console.error(`no kaplay bundle at ${kaplayPath}`);
        console.error("pass --kaplay <path to kaplay.js> or build it with npm run build in the kaplay checkout");
        process.exit(2);
    }
    const binaries = buildHosts(options);
    const result = gather(options, binaries, kaplayPath);
    result.baseline = options.mode === "baseline";

    const resultsDir = join(repo, "benchmarks", "results");
    const baselinesDir = join(repo, "benchmarks", "baselines");
    mkdirSync(resultsDir, { recursive: true });
    mkdirSync(baselinesDir, { recursive: true });
    const engines = Object.keys(result.engines);
    const engineLabel = engines.length > 1 ? "both" : engines[0];
    const resultPath = join(resultsDir, `kaplay-${platform}-${engineLabel}.json`);
    writeFileSync(resultPath, `${JSON.stringify(result, null, 2)}\n`);
    console.log(`\nwrote ${resultPath}`);

    let failures = 0;
    if (options.mode === "baseline") {
        const baselinePath = join(baselinesDir, `kaplay-${platform}-${engineLabel}.json`);
        writeFileSync(baselinePath, `${JSON.stringify(result, null, 2)}\n`);
        console.log(`wrote ${baselinePath}`);
        return;
    }

    let baselinePath = join(
        baselinesDir,
        `kaplay-${platform}-${engineLabel}.json`,
    );
    if (!existsSync(baselinePath)) {
        const combined = join(baselinesDir, `kaplay-${platform}-both.json`);
        if (existsSync(combined)) baselinePath = combined;
    }
    if (!existsSync(baselinePath)) {
        console.error(`no baseline at ${baselinePath}; run the baseline mode first`);
        process.exit(2);
    }
    const baseline = JSON.parse(readFileSync(baselinePath, "utf8"));
    failures += checkChecks(baseline, result, "baseline");
    for (const engine of engines) {
        if (!baseline.engines[engine]) continue;
        console.log(`\n${engine}:`);
        failures += report(
            timingMetrics(baseline.engines[engine]),
            timingMetrics(result.engines[engine]),
        );
    }
    if (failures > 0) process.exit(1);
}

main();
