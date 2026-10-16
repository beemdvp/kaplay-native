const params = new URLSearchParams(location.search);
const scenario = params.get("scenario") ?? "idle";
const count = Number(params.get("count") ?? 1000);
const frames = Number(params.get("frames") ?? 2000);
const warmup = Number(params.get("warmup") ?? 60);

let sink = [];
let frame = 0;
let checksum = 0;
let work = 0;
let last = performance.now();
const deltas = [];

function payload(index) {
    return {
        x: index,
        y: index * 2,
        tag: "item-" + (index % 97),
        nested: { a: index % 13, b: index / 7 },
    };
}

function allocBatch() {
    for (let index = 0; index < count; index++) {
        sink.push(payload(index));
    }
}

function jsonBatch() {
    for (let index = 0; index < count; index++) {
        const text = JSON.stringify({ index, tag: "item-" + (index % 97), nested: { a: index % 13 } });
        checksum += JSON.parse(text).tag.length;
    }
}

function stringBatch() {
    let text = "";
    for (let index = 0; index < count; index++) {
        text += index.toString(36) + ",";
    }
    checksum += text.length;
}

function timerBatch() {
    for (let index = 0; index < count; index++) {
        setTimeout(() => {
            work++;
        }, 0);
    }
}

function rafBatch() {
    for (let index = 0; index < count; index++) {
        requestAnimationFrame(() => {
            work++;
        });
    }
}

function microtaskBatch() {
    for (let index = 0; index < count; index++) {
        Promise.resolve(index).then((value) => {
            checksum += value & 1;
        });
    }
}

const canvas = scenario === "canvas" ? document.createElement("canvas") : null;
const context = canvas ? canvas.getContext("2d") : null;
if (canvas) {
    canvas.width = 512;
    canvas.height = 512;
    context.font = "16px sans-serif";
    context.fillStyle = "#44cc88";
}

function canvasBatch() {
    for (let index = 0; index < count; index++) {
        context.fillText("wave " + index, 8 + (index % 16) * 30, 16 + (index % 30) * 16);
    }
}

function runScenario() {
    switch (scenario) {
        case "idle":
            work++;
            break;
        case "alloc":
            allocBatch();
            if (sink.length > count * 4) sink = [];
            break;
        case "retain":
            allocBatch();
            if (frame % 240 === 239) sink = [];
            break;
        case "churn":
            sink = [];
            allocBatch();
            break;
        case "burst":
            if (frame % 120 < 20) {
                for (let round = 0; round < 8; round++) allocBatch();
            } else {
                allocBatch();
            }
            if (sink.length > count * 10) sink = [];
            break;
        case "strings":
            stringBatch();
            break;
        case "timers":
            timerBatch();
            break;
        case "raf":
            rafBatch();
            break;
        case "microtasks":
            microtaskBatch();
            break;
        case "json":
            jsonBatch();
            break;
        case "canvas":
            canvasBatch();
            break;
        case "codegen": {
            for (let index = 0; index < count; index++) {
                const compiled = new Function("a", "b", `return a + b + ${index};`);
                checksum += compiled(1, 2) & 1;
            }
            break;
        }
        case "gc":
            allocBatch();
            if (typeof gc === "function" && frame % 30 === 0) gc();
            if (sink.length > count * 2) sink = [];
            break;
        case "mix":
            allocBatch();
            jsonBatch();
            timerBatch();
            if (sink.length > count * 4) sink = [];
            break;
        default:
            throw new Error("unknown scenario: " + scenario);
    }
}

requestAnimationFrame(function tick(now) {
    const delta = now - last;
    last = now;
    frame++;
    if (scenario !== "raf") runScenario();
    else if (frame > warmup) runScenario();

    if (frame > warmup) deltas.push({ frame, ms: delta });

    if (frame >= frames + warmup) {
        const values = deltas.map((entry) => entry.ms);
        const sorted = values.slice().sort((a, b) => a - b);
        const at = (q) => sorted[Math.min(sorted.length - 1, Math.round((sorted.length - 1) * q))];
        const avg = values.reduce((a, b) => a + b, 0) / values.length;
        const over = (limit) => values.filter((value) => value > limit).length;
        const worst = deltas
            .slice()
            .sort((a, b) => b.ms - a.ms)
            .slice(0, 6)
            .map((entry) => `${entry.frame}:${entry.ms.toFixed(2)}`)
            .join(",");
        console.log(
            `STRESS scenario=${scenario} frames=${values.length} count=${count} avg=${avg.toFixed(3)} p50=${at(0.5).toFixed(3)} p90=${at(0.9).toFixed(3)} p99=${at(0.99).toFixed(3)} p999=${at(0.999).toFixed(3)} max=${sorted[sorted.length - 1].toFixed(3)} over16=${over(16.7)} over33=${over(33.3)} over50=${over(50)} worst=${worst} checksum=${checksum}`
        );
        return;
    }
    requestAnimationFrame(tick);
});
