const params = new URLSearchParams(location.search);
const scenario = params.get("scenario") ?? "idle";
const count = Number(params.get("count") ?? 400);
const measureFrames = Number(params.get("frames") ?? 520);

function record(name, value) {
    console.log(`BENCH kaplay.${scenario}.${name}=${value}`);
}

function recordInfo(name, value) {
    console.log(`BENCH_INFO kaplay.${scenario}.${name}=${value}`);
}

function checksum() {
    let total = 0;
    const objects = get("*", { recursive: true });
    for (const obj of objects) {
        const x = obj.pos?.x ?? 0;
        const y = obj.pos?.y ?? 0;
        total = (total * 31 + (x * 3 + y * 7) | 0) >>> 0;
    }
    return total;
}

function spread(i, columns) {
    return [
        24 + (i * 37) % 912,
        24 + Math.floor(i / columns) * 29 % 492,
    ];
}

let frameCount = 0;
let batch = [];
let fired = 0;
let microPositions = [];
let microMs = 0;

kaplay({
    width: 960,
    height: 540,
    background: [16, 20, 28],
    debug: false,
});

loadBean();

onLoad(() => {
    const started = performance.now();

    if (scenario === "draw" || scenario === "move") {
        for (let i = 0; i < count; i++) {
            const [x, y] = spread(i, 30);
            const obj = add([sprite("bean"), pos(x, y), anchor("center")]);
            if (scenario !== "draw") {
                obj.onUpdate(() => {
                    obj.pos.x += 2;
                    if (obj.pos.x > 936) obj.pos.x -= 912;
                });
            }
        }
    }

    if (scenario === "text") {
        for (let i = 0; i < count; i++) {
            const [x, y] = spread(i, 20);
            add([
                text(`score ${100000 + i} wave`, { size: 20 }),
                pos(x, y),
            ]);
        }
    }

    if (scenario === "collide") {
        for (let i = 0; i < count; i++) {
            const [x, y] = spread(i, 20);
            add([
                circle(16),
                pos(x, y),
                area(),
                body({ isStatic: true }),
                "solid",
            ]);
        }
    }

    if (scenario === "events") {
        for (let i = 0; i < count; i++) {
            const obj = add([pos(0, 0)]);
            for (let listener = 0; listener < 3; listener++) {
                obj.onUpdate(() => {
                    fired++;
                });
            }
        }
    }

    if (scenario === "micromatrix" || scenario === "microquad" || scenario === "microsprite") {
        for (let i = 0; i < count; i++) {
            microPositions.push(vec2(24 + (i * 37) % 912, 24 + (i * 53) % 492));
        }
        onDraw(() => {
            const startedDraw = performance.now();
            for (let i = 0; i < count; i++) {
                if (scenario === "micromatrix") {
                    pushTransform();
                    pushTranslate(microPositions[i]);
                    popTransform();
                }
                else if (scenario === "microquad") {
                    drawUVQuad({
                        width: 12,
                        height: 12,
                        pos: microPositions[i],
                        anchor: "center",
                    });
                }
                else {
                    drawSprite({
                        sprite: "bean",
                        pos: microPositions[i],
                        anchor: "center",
                    });
                }
            }
            microMs += performance.now() - startedDraw;
        });
    }

    recordInfo("setup_ms", (performance.now() - started).toFixed(3));
});

onUpdate(() => {
    frameCount++;

    if (scenario === "churn" && frameCount % 2 === 0) {
        for (const obj of batch) obj.destroy();
        batch = [];
        for (let i = 0; i < count / 8; i++) {
            batch.push(add([
                rect(6, 6),
                color(255, 128, 0),
                pos(24 + (i * 17) % 900, 24 + (frameCount * 7) % 480),
            ]));
        }
    }

    if (frameCount !== measureFrames) return;

    record("objects", get("*", { recursive: true }).length);
    record("checksum", checksum());
    if (scenario === "events") record("fired", fired);
    recordInfo("frames", frameCount);
    if (microMs > 0) recordInfo("loop_ms", (microMs / frameCount).toFixed(3));
});
