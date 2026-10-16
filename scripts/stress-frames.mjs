import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "..");
const platform = `${process.platform}-${process.arch}`;

const scenarios = {
    idle: { count: 0 },
    alloc: { count: 2000 },
    retain: { count: 2000 },
    churn: { count: 2000 },
    burst: { count: 2000 },
    strings: { count: 4000 },
    timers: { count: 1500 },
    raf: { count: 1500 },
    microtasks: { count: 4000 },
    json: { count: 2000 },
    canvas: { count: 16 },
    codegen: { count: 32 },
    gc: { count: 2000 },
    mix: { count: 1500 },
};

function usage() {
    console.log(`kaplay-native frame stress

  node scripts/stress-frames.mjs run [options]   run the stress scenarios

Options:
  --engine <name>    quickjs, v8, or both (default both)
  --scenario <name>  limit to one or more scenarios (repeatable)
  --frames <n>       measured frames per scenario (default 2000)
  --warmup <n>       frames dropped before measuring (default 60)
  --count <n>        override the per-scenario object count
  --repeats <n>      runs per scenario, lowest p99 wins (default 1)
  --vsync            measure with vsync on; reports missed refresh ticks
  --host <path>      host binary for the quickjs engine
  --v8-host <path>   host binary for the v8 engine
  --no-build         reuse target/release/kaplay-native-host-<engine>
  --json <path>      write the collected results (default benchmarks/results/frame-stress.json)
  --verbose          print host output
`);
}

function parse(argv) {
    const options = {
        mode: argv[0] ?? "run",
        engine: "both",
        scenario: [],
        frames: 2000,
        warmup: 60,
        count: null,
        repeats: 1,
        vsync: false,
        build: true,
        json: join(repo, "benchmarks", "results", "frame-stress.json"),
    };
    for (let index = 1; index < argv.length; index++) {
        const arg = argv[index];
        const next = argv[index + 1];
        switch (arg) {
            case "--engine":
                options.engine = next;
                index++;
                break;
            case "--scenario":
                options.scenario.push(next);
                index++;
                break;
            case "--frames":
                options.frames = Number(next);
                index++;
                break;
            case "--warmup":
                options.warmup = Number(next);
                index++;
                break;
            case "--count":
                options.count = Number(next);
                index++;
                break;
            case "--repeats":
                options.repeats = Math.max(1, Number(next));
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
            case "--no-build":
                options.build = false;
                break;
            case "--vsync":
                options.vsync = true;
                break;
            case "--json":
                options.json = next;
                index++;
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

function percentiles(values) {
    if (values.length === 0) return null;
    const sorted = values.slice().sort((a, b) => a - b);
    const at = (q) => sorted[Math.min(sorted.length - 1, Math.round((sorted.length - 1) * q))];
    const average = values.reduce((a, b) => a + b, 0) / values.length;
    const median = at(0.5);
    const sd = Math.sqrt(
        values.reduce((sum, value) => sum + (value - average) ** 2, 0) / values.length
    );
    const deviations = values.map((value) => Math.abs(value - median)).sort((a, b) => a - b);
    const mad = deviations[Math.min(deviations.length - 1, Math.round((deviations.length - 1) * 0.5))];
    return {
        n: values.length,
        avg: average,
        p50: median,
        p90: at(0.9),
        p99: at(0.99),
        p999: at(0.999),
        max: sorted[sorted.length - 1],
        sd,
        mad,
        cv: average > 0 ? (sd / average) * 100 : 0,
        over_1_5x: values.filter((value) => value > median * 1.5).length,
        over_2x: values.filter((value) => value > median * 2).length,
        over_16_7: values.filter((value) => value > 16.7).length,
        over_33: values.filter((value) => value > 33.3).length,
        over_50: values.filter((value) => value > 50).length,
    };
}

function analyzeTrace(trace, warmup, frames, vsync) {
    const end = Math.min(trace.frame_ms.length, warmup + frames - 1);
    const cut = (values) => values.slice(warmup, end);
    const frameValues = cut(trace.frame_ms);
    const phases = {};
    for (const [name, samples] of Object.entries(trace.phases_ms)) {
        phases[name] = percentiles(cut(samples));
    }
    const frame_ms = percentiles(frameValues);
    const target = vsync && frame_ms && frame_ms.p50 > 0 ? frame_ms.p50 : null;
    const missed =
        target === null
            ? null
            : frameValues.reduce(
                  (sum, value) => sum + Math.max(0, Math.round(value / target) - 1),
                  0
              );
    const worst = trace.frame_ms
        .map((ms, index) => ({ frame: index + 1, ms }))
        .slice(warmup, end)
        .sort((a, b) => b.ms - a.ms)
        .slice(0, 6)
        .map((entry) => {
            const index = entry.frame - 1;
            let phase = "none";
            let value = -1;
            for (const [name, samples] of Object.entries(trace.phases_ms)) {
                if (samples[index] > value) {
                    value = samples[index];
                    phase = name;
                }
            }
            return { frame: entry.frame, ms: entry.ms, phase, phase_ms: value };
        });
    return { frame_ms, phases, worst, target, missed };
}

function stressLine(output) {
    const match = /STRESS (.*)$/m.exec(output);
    if (!match) return null;
    const values = {};
    for (const pair of match[1].trim().split(/\s+/)) {
        const index = pair.indexOf("=");
        if (index > 0) values[pair.slice(0, index)] = pair.slice(index + 1);
    }
    return values;
}

function runScenario(binary, name, config, options) {
    const count = options.count ?? config.count;
    const frames = options.frames;
    const warmup = options.warmup;
    const benchPath = join(tmpdir(), `stress-frames-${process.pid}-${name}-bench.json`);
    const tracePath = join(tmpdir(), `stress-frames-${process.pid}-${name}-trace.json`);
    const url = `http://127.0.0.1:1/index.html?scenario=${name}&count=${count}&frames=${frames}&warmup=${warmup}`;
    const args = [
        "--script",
        "benchmarks/frame-stress/main.js",
        "--dir",
        "benchmarks/frame-stress",
        "--window",
        "640x360",
        "--frames",
        String(frames + warmup + 30),
    ];
    if (!options.vsync) args.push("--no-vsync");
    args.push(
        "--no-audio",
        "--headless",
        "--x11",
        "--url",
        url,
        "--bench-json",
        benchPath,
        "--frame-trace",
        tracePath
    );
    const result = run(binary, args);
    const output = `${result.stdout}\n${result.stderr}`;
    if (options.verbose) console.log(output.trim());
    const parsed = { status: result.status, stress: stressLine(output), bench: null, trace: null };
    if (existsSync(benchPath)) {
        parsed.bench = JSON.parse(readFileSync(benchPath, "utf8"));
        rmSync(benchPath, { force: true });
    }
    if (existsSync(tracePath)) {
        parsed.trace = JSON.parse(readFileSync(tracePath, "utf8"));
        rmSync(tracePath, { force: true });
    }
    return parsed;
}

function score(runResult) {
    return runResult.analysis?.frame_ms?.p99 ?? Infinity;
}

function gather(options, binaries) {
    const names = options.scenario.length > 0 ? options.scenario : Object.keys(scenarios);
    for (const name of names) {
        if (!scenarios[name]) throw new Error(`unknown scenario: ${name}`);
    }
    const result = {
        created: new Date().toISOString(),
        platform,
        frames: options.frames,
        warmup: options.warmup,
        vsync: options.vsync,
        engines: {},
    };
    for (const [engine, binary] of Object.entries(binaries)) {
        const entry = { scenarios: {} };
        for (const name of names) {
            const runs = [];
            for (let repeat = 0; repeat < options.repeats; repeat++) {
                console.log(
                    `stress ${name} (${engine}, ${options.count ?? scenarios[name].count} objects, ${repeat + 1}/${options.repeats})`
                );
                const runResult = runScenario(binary, name, scenarios[name], options);
                if (runResult.trace) {
                    runResult.analysis = analyzeTrace(
                        runResult.trace,
                        options.warmup,
                        options.frames,
                        options.vsync
                    );
                }
                delete runResult.trace;
                runs.push(runResult);
            }
            const best = runs.reduce((a, b) => (score(a) < score(b) ? a : b));
            entry.scenarios[name] = best;
        }
        result.engines[engine] = entry;
    }
    return result;
}

function fixed(value, width = 7, digits = 2) {
    return Number.isFinite(value) ? value.toFixed(digits).padStart(width) : "-".padStart(width);
}

function scenarioNames(results) {
    const names = new Set();
    for (const entry of Object.values(results.engines)) {
        for (const name of Object.keys(entry.scenarios)) names.add(name);
    }
    return [...names];
}

function printReport(results) {
    console.log("");
    console.log(
        [
            "scenario".padEnd(12),
            "engine".padEnd(8),
            "p50".padStart(7),
            "p99".padStart(7),
            "p99.9".padStart(7),
            "max".padStart(8),
            "sd".padStart(7),
            "cv%".padStart(6),
            "1.5x".padStart(5),
            "missed".padStart(6),
            ">16.7".padStart(6),
            "worst frame (ms, phase)",
        ].join(" ")
    );
    for (const name of scenarioNames(results)) {
        for (const [engine, entry] of Object.entries(results.engines)) {
            const scenario = entry.scenarios[name];
            const frame = scenario?.analysis?.frame_ms;
            if (!frame) {
                console.log(`${name.padEnd(12)} ${engine.padEnd(8)} no data (status ${scenario?.status})`);
                continue;
            }
            const missed = scenario.analysis.missed;
            const worst = scenario.analysis.worst
                .slice(0, 3)
                .map((hit) => `${hit.frame}:${hit.ms.toFixed(1)}(${hit.phase})`)
                .join(" ");
            console.log(
                [
                    name.padEnd(12),
                    engine.padEnd(8),
                    fixed(frame.p50),
                    fixed(frame.p99),
                    fixed(frame.p999),
                    fixed(frame.max, 8, 1),
                    fixed(frame.sd),
                    `${frame.cv.toFixed(1)}%`.padStart(6),
                    String(frame.over_1_5x).padStart(5),
                    (missed === null ? "-" : String(missed)).padStart(6),
                    String(frame.over_16_7).padStart(6),
                    worst,
                ].join(" ")
            );
        }
    }
}

function printComparison(results) {
    const engines = Object.keys(results.engines);
    if (engines.length < 2) return;
    console.log("");
    console.log("v8 vs quickjs (frame time, lower is better)");
    const flags = [];
    for (const name of scenarioNames(results)) {
        const quickjsAnalysis = results.engines.quickjs?.scenarios[name]?.analysis;
        const v8Analysis = results.engines.v8?.scenarios[name]?.analysis;
        const quickjs = quickjsAnalysis?.frame_ms;
        const v8 = v8Analysis?.frame_ms;
        if (!quickjs || !v8) continue;
        const maxRatio = quickjs.max > 0 ? v8.max / quickjs.max : 1;
        const hitches = v8.over_16_7 - quickjs.over_16_7;
        const note = [];
        if (maxRatio > 1.25 && v8.max - quickjs.max > 2) {
            note.push(`max x${maxRatio.toFixed(2)}`);
        }
        if (v8.cv > quickjs.cv * 1.5 && v8.cv - quickjs.cv > 5) {
            note.push(`cv ${v8.cv.toFixed(1)}% vs ${quickjs.cv.toFixed(1)}%`);
        }
        if (hitches > 0) note.push(`+${hitches} over 16.7ms`);
        if (quickjsAnalysis.missed !== null && v8Analysis.missed > quickjsAnalysis.missed) {
            note.push(`missed ${v8Analysis.missed} vs ${quickjsAnalysis.missed}`);
        }
        if (note.length > 0) flags.push(`${name}: ${note.join(", ")}`);
        const missed =
            v8Analysis.missed === null
                ? ""
                : `  missed ${v8Analysis.missed} vs ${quickjsAnalysis.missed}`;
        console.log(
            `${name.padEnd(12)} max ${fixed(v8.max, 8, 1)} vs ${fixed(quickjs.max, 8, 1)}  p99 ${fixed(v8.p99)} vs ${fixed(quickjs.p99)}  cv ${fixed(v8.cv, 6, 1)}% vs ${fixed(quickjs.cv, 6, 1)}%  hitches ${v8.over_16_7} vs ${quickjs.over_16_7}${missed}`
        );
    }
    console.log("");
    if (flags.length === 0) {
        console.log("no v8-specific frame-time regressions found");
    } else {
        console.log(`v8 frame-time outliers: ${flags.join("; ")}`);
    }
}

function main() {
    const options = parse(process.argv.slice(2));
    if (options.mode !== "run") {
        usage();
        process.exit(2);
    }
    const binaries = buildHosts(options);
    const results = gather(options, binaries);
    printReport(results);
    printComparison(results);
    mkdirSync(dirname(options.json), { recursive: true });
    writeFileSync(options.json, `${JSON.stringify(results, null, 2)}\n`);
    console.log(`\nresults written to ${options.json}`);
}

main();
