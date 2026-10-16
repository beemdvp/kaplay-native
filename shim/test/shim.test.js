import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import vm from "node:vm";

import { installStub } from "./stub.js";

const here = dirname(fileURLToPath(import.meta.url));
const stub = installStub();
vm.runInThisContext(readFileSync(join(here, "..", "src", "shim.js"), "utf8"));

function advance(ms) {
    stub.state.time += ms;
    globalThis.__kapNative_onFrame(stub.state.time);
}

function drain() {
    globalThis.__kapNative_onFrame(stub.state.time);
}

function makeCanvas(width = 320, height = 240) {
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    document.body.appendChild(canvas);
    return canvas;
}

test("timers and animation frames run in order", () => {
    const order = [];
    setTimeout(() => order.push("timeout"), 10);
    requestAnimationFrame(() => order.push("frame1"));
    requestAnimationFrame(() => order.push("frame2"));
    const interval = setInterval(() => order.push("interval"), 5);

    drain();
    assert.deepEqual(order, ["frame1", "frame2"]);

    advance(10);
    assert.deepEqual(order, ["frame1", "frame2", "interval", "timeout"]);

    clearInterval(interval);
    advance(10);
    assert.deepEqual(order, ["frame1", "frame2", "interval", "timeout"]);
});

test("timers fire in due order even when scheduled out of order", () => {
    const order = [];
    setTimeout(() => order.push("late"), 20);
    setTimeout(() => order.push("early"), 5);
    advance(25);
    assert.deepEqual(order, ["early", "late"]);
});

test("cancelAnimationFrame removes a queued callback", () => {
    let called = false;
    const id = requestAnimationFrame(() => {
        called = true;
    });
    cancelAnimationFrame(id);
    drain();
    assert.equal(called, false);
});

test("keyboard events reach canvas listeners", () => {
    const canvas = makeCanvas();
    const received = [];
    canvas.addEventListener("keydown", (event) => received.push(event));
    globalThis.__kapNative_dispatch(
        "keydown",
        JSON.stringify({ key: "ArrowLeft", code: "ArrowLeft", repeat: true, capsLock: true }),
    );
    assert.equal(received.length, 1);
    assert.equal(received[0].key, "ArrowLeft");
    assert.equal(received[0].code, "ArrowLeft");
    assert.equal(received[0].repeat, true);
    assert.equal(received[0].getModifierState("CapsLock"), true);
    assert.equal(received[0].defaultPrevented, false);
    received[0].preventDefault();
    assert.equal(received[0].defaultPrevented, true);
});

test("mouse offsets are relative to the drawn canvas", () => {
    const canvas = makeCanvas(960, 540);
    const events = [];
    canvas.addEventListener("mousemove", (event) => events.push(event));
    globalThis.__kapNative_dispatch(
        "mousemove",
        JSON.stringify({ clientX: 480, clientY: 270, movementX: 2, movementY: -3 }),
    );
    assert.equal(events[0].offsetX, 480);
    assert.equal(events[0].offsetY, 270);
    assert.equal(events[0].movementX, 2);
    assert.equal(events[0].movementY, -3);
});

test("fixed size canvases are letterboxed in the window", () => {
    const canvas = makeCanvas();
    canvas.style.cssText = "cursor: default;width: 320px;height: 240px";
    const events = [];
    canvas.addEventListener("mousedown", (event) => events.push(event));
    canvas.addEventListener("mouseup", (event) => events.push(event));
    globalThis.__kapNative_dispatch("mousedown", JSON.stringify({ clientX: 480, clientY: 270, button: 0 }));

    const rect = canvas.getBoundingClientRect();
    assert.equal(rect.x, 120);
    assert.equal(rect.y, 0);
    assert.equal(rect.width, 720);
    assert.equal(rect.height, 540);
    assert.equal(events[0].offsetX, 160);
    assert.equal(events[0].offsetY, 120);
    assert.equal(events[0].buttons, 1);

    globalThis.__kapNative_dispatch("mouseup", JSON.stringify({ clientX: 480, clientY: 270, button: 0 }));
    assert.equal(events[1].buttons, 0);
});

test("resize updates layout and notifies observers", () => {
    const canvas = makeCanvas();
    const observed = [];
    const observer = new ResizeObserver((entries) => observed.push(entries));
    observer.observe(canvas);
    let windowResized = 0;
    window.addEventListener("resize", () => windowResized++);

    globalThis.__kapNative_dispatch(
        "resize",
        JSON.stringify({ width: 640, height: 480, framebufferWidth: 640, framebufferHeight: 480, dpr: 1 }),
    );

    assert.equal(window.innerWidth, 640);
    assert.equal(window.innerHeight, 480);
    assert.equal(canvas.offsetWidth, 640);
    assert.equal(windowResized, 1);
    assert.equal(observed.length, 1);
    assert.equal(observed[0][0].target, canvas);
});

test("style cursor changes drive the native cursor", () => {
    const canvas = makeCanvas();
    canvas.style.cursor = "none";
    let call = stub.calls.filter((entry) => entry.name === "window.setCursor").at(-1);
    assert.deepEqual(call.args, ["none"]);
    canvas.style.cssText = "cursor: pointer; width: 100%";
    call = stub.calls.filter((entry) => entry.name === "window.setCursor").at(-1);
    assert.deepEqual(call.args, ["pointer"]);
    assert.equal(canvas.style.cursor, "pointer");
    assert.equal(canvas.style.width, "100%");
});

test("mutation observers observe, disconnect and take records", () => {
    const observer = new MutationObserver(() => {});
    observer.observe(document);
    assert.deepEqual(observer.takeRecords(), []);
    observer.disconnect();
    assert.deepEqual(observer.takeRecords(), []);
});

test("canvas backing store resizes the drawing buffer", () => {
    const canvas = makeCanvas();
    const gl = canvas.getContext("webgl");
    canvas.width = 640;
    const resize = stub.calls.filter((call) => call.name === "gl.resizeDrawingBuffer").at(-1);
    assert.deepEqual(resize.args, [640, 240]);
    assert.equal(gl.drawingBufferWidth, 320);
});

test("images load from data urls and fire onload asynchronously", () => {
    const image = new Image();
    let loaded = null;
    image.onload = () => {
        loaded = [image.width, image.height];
    };
    image.src = "data:image/png;base64,AQIDBA==";
    assert.equal(loaded, null);
    drain();
    assert.deepEqual(loaded, [4, 1]);
});

test("images report errors for unknown sources", () => {
    const image = new Image();
    let error = null;
    image.onerror = () => {
        error = true;
    };
    image.src = "missing.png";
    drain();
    assert.equal(error, true);
});

test("local storage persists through the native bridge", () => {
    window.localStorage["highscore"] = JSON.stringify(4200);
    assert.equal(window.localStorage["highscore"], "4200");
    const saved = stub.calls.filter((call) => call.name === "storage.save").at(-1);
    assert.equal(saved.args[0], '{"highscore":"4200"}');
    assert.equal(window.localStorage.getItem("highscore"), "4200");
    window.localStorage.removeItem("highscore");
    assert.equal(window.localStorage.getItem("highscore"), null);
});

test("fonts register bytes and check families", async () => {
    const face = new FontFace("MyFont", new Uint8Array([9, 9, 9]));
    await face.load();
    assert.equal(face.status, "loaded");
    const registered = stub.calls.filter((call) => call.name === "font.register").at(-1);
    assert.deepEqual(registered.args, ["MyFont", 3]);
    assert.equal(document.fonts.check("64px MyFont"), true);
    assert.equal(document.fonts.check("64px monospace"), true);
    assert.equal(document.fonts.check("64px Unknown"), false);

    const urlFace = new FontFace("Loaded", "url(font.ttf)");
    await urlFace.load();
    const urlRegistered = stub.calls.filter((call) => call.name === "font.register").at(-1);
    assert.deepEqual(urlRegistered.args, ["Loaded", 4]);
});

test("webgl texture uploads dispatch on the source type", () => {
    const canvas = makeCanvas();
    const gl = canvas.getContext("webgl");

    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 16, 8, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    let call = stub.calls.filter((entry) => entry.name === "gl.texImage2D").at(-1);
    assert.deepEqual(call.args, [3553, 6408, 16, 8, 6408, 5121, null]);

    gl.texImage2D(
        gl.TEXTURE_2D,
        0,
        gl.RGBA,
        2,
        2,
        0,
        gl.RGBA,
        gl.UNSIGNED_BYTE,
        new Uint8Array([1, 2, 3, 4]),
    );
    call = stub.calls.filter((entry) => entry.name === "gl.texImage2D").at(-1);
    assert.deepEqual(call.args.slice(0, 6), [3553, 6408, 2, 2, 6408, 5121]);
    assert.equal(Array.from(call.args[6]).join(","), "1,2,3,4");

    const image = new Image();
    image.src = "data:image/png;base64,AQIDBA==";
    drain();
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, image);
    call = stub.calls.filter((entry) => entry.name === "gl.texImage2DImage").at(-1);
    assert.equal(call.args.length, 5);
    assert.deepEqual(call.args.slice(0, 4), [3553, 6408, 6408, 5121]);

    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, canvas);
    assert.ok(stub.calls.some((entry) => entry.name === "gl.texImage2DCanvas"));

    gl.texSubImage2D(gl.TEXTURE_2D, 0, 3, 4, gl.RGBA, gl.UNSIGNED_BYTE, image);
    call = stub.calls.filter((entry) => entry.name === "gl.texSubImage2DImage").at(-1);
    assert.deepEqual(call.args, [3553, 3, 4, 6408, 5121, image.__id]);

    gl.texSubImage2D(gl.TEXTURE_2D, 0, 1, 2, gl.RGBA, gl.UNSIGNED_BYTE, canvas);
    call = stub.calls.filter((entry) => entry.name === "gl.texSubImage2DCanvas").at(-1);
    assert.deepEqual(call.args, [3553, 1, 2, 6408, 5121, canvas.__id]);
});

test("webgl buffers and uniforms normalize their arguments", () => {
    const canvas = makeCanvas();
    const gl = canvas.getContext("webgl");

    gl.bufferData(gl.ARRAY_BUFFER, 128, gl.DYNAMIC_DRAW);
    assert.ok(stub.calls.some((entry) => entry.name === "gl.bufferDataSize"));

    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([1, 2]), gl.STATIC_DRAW);
    let call = stub.calls.filter((entry) => entry.name === "gl.bufferData").at(-1);
    assert.equal(call.args[1].length, 8);

    gl.bufferSubData(gl.ARRAY_BUFFER, 4, new Uint16Array([7, 8]));
    call = stub.calls.filter((entry) => entry.name === "gl.bufferSubData").at(-1);
    assert.deepEqual(call.args.slice(0, 2), [34962, 4]);
    assert.equal(call.args[2].length, 4);

    gl.uniform1fv(null, [1, 2, 3]);
    call = stub.calls.filter((entry) => entry.name === "gl.uniform1fv").at(-1);
    assert.deepEqual(call.args, [-1, new Float32Array([1, 2, 3])]);

    gl.uniformMatrix4fv(5, false, new Float32Array(16));
    call = stub.calls.filter((entry) => entry.name === "gl.uniformMatrix4fv").at(-1);
    assert.equal(call.args[0], 5);
    assert.equal(call.args[1].length, 16);

    assert.equal(gl.getUniformLocation(1, "missing"), null);
    assert.ok(gl.getUniformLocation(1, "present") > 0);
    assert.equal(gl.getExtension("WEBGL_lose_context").loseContext instanceof Function, true);
    assert.equal(gl.getExtension("EXT_texture_filter_anisotropic"), null);
});

test("canvas 2d contexts proxy to native calls", () => {
    const canvas = document.createElement("canvas");
    canvas.width = 64;
    canvas.height = 32;
    const context = canvas.getContext("2d");
    context.fillStyle = "#ff8800";
    context.font = "24px monospace";
    context.textBaseline = "top";
    context.textAlign = "left";
    context.lineWidth = 3;
    context.lineJoin = "round";
    context.fillText("hi", 1, 2);
    context.strokeText("hi", 1, 2);
    context.clearRect(0, 0, 8, 8);

    const fill = stub.calls.filter((entry) => entry.name === "canvas.setFillStyle").at(-1);
    assert.equal(fill.args[1], "#ff8800");
    const font = stub.calls.filter((entry) => entry.name === "canvas.setFont").at(-1);
    assert.equal(font.args[1], "24px monospace");
    const baseline = stub.calls.filter((entry) => entry.name === "canvas.setTextBaseline").at(-1);
    assert.equal(baseline.args[1], "top");

    const metrics = context.measureText("hello");
    assert.deepEqual(metrics, {
        width: 35,
        actualBoundingBoxLeft: 0,
        actualBoundingBoxRight: 35,
        actualBoundingBoxAscent: 10,
        actualBoundingBoxDescent: 2,
    });

    const pixels = context.getImageData(0, 0, 2, 2);
    assert.ok(pixels instanceof ImageData);
    assert.ok(pixels.data instanceof Uint8ClampedArray);
    assert.equal(pixels.width, 2);
    assert.equal(pixels.height, 2);
    assert.equal(pixels.data[10], 10);

    context.putImageData(pixels, 4, 5);
    const put = stub.calls.filter((entry) => entry.name === "canvas.putImageData").at(-1);
    assert.deepEqual(put.args.slice(0, 5), [put.args[0], 4, 5, 2, 2]);
    assert.ok(put.args[5] instanceof Uint8Array);
});

test("canvas 2d drawImage forwards the right number of arguments", () => {
    const canvas = document.createElement("canvas");
    canvas.width = 64;
    canvas.height = 64;
    const context = canvas.getContext("2d");
    const image = new Image();
    image.src = "data:image/png;base64,AQIDBA==";
    drain();

    context.drawImage(image, 10, 20);
    let call = stub.calls.filter((entry) => entry.name === "canvas.drawImage").at(-1);
    assert.deepEqual(call.args.slice(1, 3), [0, image.__id]);
    assert.equal(call.args.length, 11);
    assert.deepEqual(call.args.slice(3, 11), [0, 0, 4, 1, 10, 20, 4, 1]);

    context.drawImage(image, 1, 2, 3, 4, 5, 6, 7, 8);
    call = stub.calls.filter((entry) => entry.name === "canvas.drawImage").at(-1);
    assert.deepEqual(call.args.slice(3, 11), [1, 2, 3, 4, 5, 6, 7, 8]);
});

test("canvas screenshots use the native image path", () => {
    const canvas = document.createElement("canvas");
    canvas.width = 4;
    canvas.height = 4;
    document.body.appendChild(canvas);
    const url = canvas.toDataURL();
    assert.ok(url.startsWith("data:image/png;base64,"));

    const glCanvas = document.createElement("canvas");
    glCanvas.width = 4;
    glCanvas.height = 4;
    document.body.appendChild(glCanvas);
    glCanvas.getContext("webgl");
    assert.ok(glCanvas.toDataURL().startsWith("data:image/png;base64,"));
    assert.ok(stub.calls.some((entry) => entry.name === "gl.screenshotLength"));
    assert.ok(stub.calls.some((entry) => entry.name === "gl.copyScreenshot"));
});

test("the platform namespace exposes steam", () => {
    assert.equal(kaplayNative.platform, "test");
    assert.equal(kaplayNative.version, "test");
    assert.equal(kaplayNative.steam.available(), true);
    assert.equal(kaplayNative.steam.playerName(), "Tester");
    assert.equal(kaplayNative.steam.steamId(), "76561198000000000");

    assert.equal(kaplayNative.steam.unlockAchievement("FIRST"), true);
    assert.equal(kaplayNative.steam.unlockAchievement("locked"), false);
    assert.equal(kaplayNative.steam.isAchievementUnlocked("unlocked"), true);
    assert.equal(kaplayNative.steam.storeStats(), true);

    assert.equal(kaplayNative.steam.setStatInt("deaths", 3), true);
    assert.equal(kaplayNative.steam.getStatInt("deaths"), 7);
    assert.equal(kaplayNative.steam.getStatInt("missing"), null);
    assert.equal(kaplayNative.steam.setStatFloat("time", 2.5), true);
    assert.equal(kaplayNative.steam.getStatFloat("time"), 2.5);

    assert.equal(kaplayNative.steam.writeCloud("save.json", "{}"), true);
    assert.equal(kaplayNative.steam.readCloud("save.json"), "cloud-data");
    assert.equal(kaplayNative.steam.readCloud("missing"), null);
    assert.equal(kaplayNative.steam.cloudExists("save.json"), true);
    assert.equal(kaplayNative.steam.deleteCloud("save.json"), true);

    kaplayNative.steam.runCallbacks();
    assert.ok(stub.calls.some((entry) => entry.name === "steam.runCallbacks"));
    assert.ok(stub.calls.some((entry) => entry.name === "steam.setStatInt" && entry.args[1] === 3));
});

test("url and search params follow the web api", () => {
    const parsed = new URL("/colyseus?x=1&y=a+b", "ws://127.0.0.1:2567");
    assert.equal(parsed.protocol, "ws:");
    assert.equal(parsed.hostname, "127.0.0.1");
    assert.equal(parsed.port, "2567");
    assert.equal(parsed.pathname, "/colyseus");
    assert.equal(parsed.search, "?x=1&y=a+b");
    assert.equal(parsed.href, "ws://127.0.0.1:2567/colyseus?x=1&y=a+b");
    assert.equal(parsed.searchParams.get("y"), "a b");
    assert.equal(parsed.toString(), parsed.href);

    const relative = new URL("../other", "wss://example.com:8443/a/b/c");
    assert.equal(relative.href, "wss://example.com:8443/a/other");
    const schemeRelative = new URL("//cdn.example.com/x", "https://example.com");
    assert.equal(schemeRelative.href, "https://cdn.example.com/x");
    const samePage = new URL("#frag", "https://example.com/a?b=1");
    assert.equal(samePage.href, "https://example.com/a?b=1#frag");

    assert.throws(() => new URL("not a url"), TypeError);

    const params = new URLSearchParams("?a=1&a=2&b=%20c");
    assert.equal(params.getAll("a").join(","), "1,2");
    assert.equal(params.get("b"), " c");
    assert.equal(params.has("a"), true);
    params.set("a", "9");
    params.append("d", "e f");
    assert.equal(params.toString(), "a=9&b=+c&d=e+f");
    params.delete("b");
    assert.equal(params.toString(), "a=9&d=e+f");
    assert.equal(new URLSearchParams({ deep: true }).get("deep"), "true");
    assert.equal(new URLSearchParams([["k", 2]]).get("k"), "2");
    assert.equal([...params.keys()].join(","), "a,d");
    assert.equal(params.size, 2);
});

test("matchMedia reports pointer capabilities", () => {
    assert.equal(window.matchMedia("(hover: none) and (pointer: coarse)").matches, false);
    assert.equal(window.matchMedia("(prefers-color-scheme: dark)").media, "(prefers-color-scheme: dark)");
});

test("location exposes the fields sdk clients parse", () => {
    assert.equal(location.protocol, "kaplay-native:");
    assert.equal(location.hostname, "game");
    assert.equal(location.port, "");
    assert.equal(location.searchParams.get("solo"), null);
});

test("crypto fills typed arrays and mints uuids", () => {
    const bytes = new Uint8Array(16);
    assert.equal(crypto.getRandomValues(bytes), bytes);
    assert.ok(bytes.some((value) => value !== 0));

    const uuid = crypto.randomUUID();
    assert.match(uuid, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    assert.notEqual(crypto.randomUUID(), uuid);

    const words = new Uint32Array(2);
    crypto.getRandomValues(words);
    assert.equal(words.length, 2);
});

test("dom elements support lookup, nesting, and input classes", () => {
    const banner = document.createElement("div");
    banner.id = "offline-banner";
    banner.textContent = "offline";
    Object.assign(banner.style, { position: "fixed", display: "none" });
    document.body.prepend(banner);

    assert.equal(document.getElementById("offline-banner"), banner);
    assert.equal(document.querySelector("#offline-banner"), banner);
    assert.equal(banner.parentElement, document.body);
    assert.equal(banner.style.display, "none");
    assert.equal(banner.style.cssText.includes("position: fixed"), true);

    const input = document.createElement("input");
    assert.ok(input instanceof HTMLInputElement);
    assert.ok(input instanceof HTMLElement);
    input.type = "password";
    input.setAttribute("autocomplete", "off");
    assert.equal(input.getAttribute("autocomplete"), "off");
    input.value = "hunter2";
    let seen = null;
    input.addEventListener("input", () => {
        seen = input.value;
    });
    input.dispatchEvent(new Event("input"));
    assert.equal(seen, "hunter2");
    assert.equal(document.getElementsByTagName("input").includes(input), false);

    const form = document.createElement("form");
    form.noValidate = true;
    form.addEventListener("submit", (event) => event.preventDefault());
    document.body.prepend(form);
    form.appendChild(input);
    assert.equal(input.parentElement, form);
    assert.equal(form.children[0], input);
    assert.ok(document.getElementsByTagName("input").includes(input));

    const event = new MouseEvent("mousemove", { clientX: 5, clientY: 6 });
    assert.equal(event.type, "mousemove");
    assert.equal(event.clientX, 5);
    document.body.dispatchEvent(event);

    input.remove();
    banner.remove();
    assert.equal(document.getElementById("offline-banner"), null);
    assert.equal(banner.parentElement, null);
    assert.equal(form.children.length, 0);
});

test("headers expose browser accessors", () => {
    const headers = new Headers({ "Content-Type": "application/json", "x-count": "2" });
    headers.append("x-count", "3");
    headers.set("X-Name", "frax");

    assert.equal(headers.get("content-type"), "application/json");
    assert.equal(headers.get("x-count"), "2, 3");
    assert.equal(headers.get("missing"), null);
    assert.equal(headers.has("X-NAME"), true);

    const seen = [];
    headers.forEach((value, key) => seen.push(`${key}=${value}`));
    assert.deepEqual(seen, ["content-type=application/json", "x-count=2, 3", "x-name=frax"]);
    assert.deepEqual(Array.from(new Headers(headers)), Array.from(headers.entries()));
});

test("fetch resolves data urls and rejects remote hosts without network access", async () => {
    const response = await fetch("data:application/json,%7B%22ok%22%3Atrue%7D");
    assert.equal(response.ok, true);
    assert.deepEqual(await response.json(), { ok: true });

    globalThis.__kapNative.env.networkAllowed = false;
    await assert.rejects(
        fetch("https://example.com/rooms"),
        /network access is disabled/,
    );
});

test("fetch performs native requests and exposes response metadata", async () => {
    globalThis.__kapNative.env.networkAllowed = true;
    stub.net.route("http://127.0.0.1:2567/colyseus/characters", {
        headers: [["content-type", "application/json"]],
        body: '{"players":3}',
    });

    const response = await fetch("http://127.0.0.1:2567/colyseus/characters", {
        headers: { authorization: "Bearer token" },
    });
    assert.equal(response.ok, true);
    assert.equal(response.status, 200);
    assert.equal(response.headers.get("content-type"), "application/json");
    assert.deepEqual(await response.json(), { players: 3 });

    const call = stub.calls.find((entry) => entry.name === "net.fetch");
    assert.deepEqual(call.args.slice(0, 4), [
        call.args[0],
        "GET",
        "http://127.0.0.1:2567/colyseus/characters",
        [["authorization", "Bearer token"]],
    ]);

    stub.net.route("http://127.0.0.1:2567/missing", {
        status: 404,
        statusText: "Not Found",
        headers: [["content-type", "text/plain"]],
        body: "nope",
    });
    const missing = await fetch("http://127.0.0.1:2567/missing");
    assert.equal(missing.ok, false);
    assert.equal(missing.status, 404);
    assert.equal(missing.statusText, "Not Found");
    assert.equal(await missing.text(), "nope");
});

test("fetch posts bodies and reports network failures", async () => {
    globalThis.__kapNative.env.networkAllowed = true;
    stub.net.route("http://127.0.0.1:2567/colyseus/matchmake/join", { body: "{}" });
    await fetch("http://127.0.0.1:2567/colyseus/matchmake/join", {
        method: "post",
        headers: new Headers([["content-type", "application/json"]]),
        body: JSON.stringify({ room: "island" }),
    });
    const call = stub.calls.filter((entry) => entry.name === "net.fetch").pop();
    assert.equal(call.args[1], "POST");
    assert.deepEqual(call.args[4], Array.from(new TextEncoder().encode('{"room":"island"}')));

    await assert.rejects(fetch("http://127.0.0.1:9/rooms"), /connection refused/);
});

test("websockets forward open, message, and close events", async () => {
    globalThis.__kapNative.env.networkAllowed = true;
    const socket = new WebSocket("ws://127.0.0.1:2567/room", { protocols: ["colyseus"] });
    assert.equal(socket.readyState, WebSocket.CONNECTING);
    assert.equal(socket.protocol, "colyseus");

    const events = [];
    socket.onopen = () => events.push("open");
    socket.onmessage = (event) => events.push(event.data);
    socket.onclose = (event) => events.push(`close:${event.code}`);
    socket.onerror = (event) => events.push(`error:${event.message}`);

    const id = stub.net.opened.at(-1);
    stub.net.open(id);
    assert.equal(socket.readyState, WebSocket.OPEN);
    assert.equal(events[0], "open");

    socket.binaryType = "arraybuffer";
    stub.net.deliver(id, "hello");
    assert.equal(events[1], "hello");
    stub.net.deliver(id, new Uint8Array([1, 2, 3]));
    assert.ok(events[2] instanceof ArrayBuffer);
    assert.deepEqual(Array.from(new Uint8Array(events[2])), [1, 2, 3]);

    socket.send("ping");
    socket.send(new Uint8Array([9, 8]));
    const sent = stub.calls.filter((entry) => entry.name.startsWith("net.send"));
    assert.deepEqual(sent.at(-2).args, [id, "ping"]);
    assert.deepEqual(sent.at(-1).args, [id, [9, 8]]);

    socket.close(4000, "reconnect");
    assert.equal(socket.readyState, WebSocket.CLOSING);
    const closed = stub.calls.find((entry) => entry.name === "net.closeSocket");
    assert.deepEqual(closed.args, [id, 4000, "reconnect"]);

    stub.net.close(id, 4000, "reconnect");
    assert.equal(socket.readyState, WebSocket.CLOSED);
    assert.equal(events.at(-1), "close:4000");
});

test("websockets fail cleanly when network access is disabled", async () => {
    globalThis.__kapNative.env.networkAllowed = false;
    const socket = new WebSocket("ws://127.0.0.1:2567/room");
    const events = [];
    socket.onerror = () => events.push("error");
    socket.onclose = (event) => events.push(`close:${event.code}`);
    const delivered = new Promise((resolve) => setTimeout(resolve, 0));
    advance(1);
    await delivered;
    assert.deepEqual(events, ["error", "close:1006"]);
    assert.equal(socket.readyState, WebSocket.CLOSED);
});

test("abort controllers cancel pending fetches", async () => {
    globalThis.__kapNative.env.networkAllowed = true;
    stub.net.route("http://127.0.0.1:2567/slow", { body: "late", defer: true });
    const controller = new AbortController();
    stub.calls.length = 0;
    const pending = fetch("http://127.0.0.1:2567/slow", { signal: controller.signal });
    controller.abort();
    await assert.rejects(pending, /aborted/);
});

test("relative requests resolve against the page url", async () => {
    globalThis.__kapNative.env.networkAllowed = true;
    const previous = {
        href: location.href,
        protocol: location.protocol,
        host: location.host,
        hostname: location.hostname,
        port: location.port,
        origin: location.origin,
    };
    location.href = "http://127.0.0.1:2567/";
    location.protocol = "http:";
    location.host = "127.0.0.1:2567";
    location.hostname = "127.0.0.1";
    location.port = "2567";
    location.origin = "http://127.0.0.1:2567";

    stub.net.route("http://127.0.0.1:2567/colyseus/characters", { body: '{"players":[]}' });
    const response = await fetch("/colyseus/characters");
    assert.deepEqual(await response.json(), { players: [] });

    stub.files.set("sprites/rabbit.png", new TextEncoder().encode("bundled"));
    const asset = fetch("sprites/rabbit.png");
    advance(1);
    const bundled = await asset;
    assert.equal(await bundled.text(), "bundled");

    const endpoint = new URL("/colyseus", "ws://127.0.0.1:2567");
    assert.equal(endpoint.href, "ws://127.0.0.1:2567/colyseus");

    Object.assign(location, previous);
});

test("location reflects the configured page url", () => {
    const page = new URL("http://127.0.0.1:2567/play/index.html?debug&instantLogin#menu");
    assert.equal(page.protocol, "http:");
    assert.equal(page.hostname, "127.0.0.1");
    assert.equal(page.port, "2567");
    assert.equal(page.search, "?debug&instantLogin");
    assert.equal(page.searchParams.get("instantLogin"), "");
    assert.equal(page.hash, "#menu");
});

test("decodeAudioData resolves a native audio buffer", async () => {
    const context = new AudioContext();
    const bytes = new TextEncoder().encode("fake audio");
    const buffer = await context.decodeAudioData(bytes.buffer);
    assert.equal(buffer.sampleRate, 48000);
    assert.equal(buffer.numberOfChannels, 2);
    assert.equal(buffer.length, 4800);
    assert.equal(buffer.getChannelData(0).length, 4800);
    assert.ok(Math.abs(buffer.duration - 0.1) < 1e-6);
});

test("buffer sources apply graph gain and fire ended", () => {
    const context = new AudioContext();
    const master = context.createGain();
    master.gain.value = 0.5;
    master.connect(context.destination);
    const gain = context.createGain();
    gain.gain.value = 0.5;
    gain.connect(master);
    const source = context.createBufferSource();
    source.buffer = context.createBuffer(1, 480, 48000);
    source.connect(gain);
    let ended = 0;
    source.onended = () => ended++;
    source.start();
    const play = stub.calls.filter((call) => call.name === "audio.play").at(-1);
    assert.ok(play);
    assert.ok(Math.abs(play.args[2] - 0.25) < 1e-6);
    assert.equal(play.args[3], 0);
    assert.equal(play.args[4], 1);
    const voice = play.args[0];

    master.gain.value = 0.25;
    const updated = stub.calls.filter((call) => call.name === "audio.setGain").at(-1);
    assert.ok(Math.abs(updated.args[1] - 0.125) < 1e-6);

    stub.audio.end(voice);
    assert.equal(ended, 1);
});

test("buffer sources mute when they do not reach the destination", () => {
    const context = new AudioContext();
    const gain = context.createGain();
    const source = context.createBufferSource();
    source.buffer = context.createBuffer(1, 64, 48000);
    source.connect(gain);
    source.start();
    const play = stub.calls.filter((call) => call.name === "audio.play").at(-1);
    assert.equal(play.args[2], 0);
});

test("music elements decode, play, seek and pause", async () => {
    stub.files.set("music/Forest.ogg", new TextEncoder().encode("oggdata"));
    const element = new Audio("music/Forest.ogg");
    assert.equal(element.paused, true);
    const playing = element.play();
    advance(1);
    await playing;
    assert.equal(element.paused, false);
    assert.equal(element.duration, 0.1);
    const play = stub.calls.filter((call) => call.name === "audio.play").at(-1);
    assert.ok(play);

    advance(500);
    assert.ok(Math.abs(element.currentTime - 0.5) < 1e-6);
    element.currentTime = 2;
    assert.equal(element.currentTime, 2);
    element.pause();
    assert.equal(element.paused, true);
    assert.ok(stub.calls.some((call) => call.name === "audio.stop"));
});

test("media element sources route through the graph", async () => {
    stub.files.set("music/Battle.ogg", new TextEncoder().encode("oggdata"));
    const context = new AudioContext();
    const master = context.createGain();
    master.gain.value = 0.5;
    master.connect(context.destination);
    const element = new Audio("music/Battle.ogg");
    element.volume = 0.5;
    const playing = element.play();
    advance(1);
    await playing;
    const before = stub.calls.filter((call) => call.name === "audio.play").at(-1);
    assert.ok(Math.abs(before.args[2] - 0.5) < 1e-6);

    const source = context.createMediaElementSource(element);
    source.connect(master);
    const after = stub.calls.filter((call) => call.name === "audio.setGain").at(-1);
    assert.ok(Math.abs(after.args[1] - 0.25) < 1e-6);
});

test("video elements open, play, and advance their clock", async () => {
    stub.files.set("video/intro.mp4", new Uint8Array([1, 2, 3, 4, 5, 6]));
    const element = document.createElement("video");
    element.src = "video/intro.mp4";
    const events = [];
    element.addEventListener("playing", () => events.push("playing"));
    element.addEventListener("timeupdate", () => events.push("timeupdate"));
    assert.equal(element.paused, true);

    const playing = element.play();
    advance(1);
    await playing;

    const opened = stub.calls.filter((call) => call.name === "video.openAsset").at(-1);
    assert.deepEqual(opened.args, ["video/intro.mp4"]);
    assert.equal(element.paused, false);
    assert.equal(element.videoWidth, 160);
    assert.equal(element.videoHeight, 120);
    assert.equal(element.duration, 1);
    assert.ok(events.includes("playing"));
    assert.ok(events.includes("timeupdate"));

    const before = element.currentTime;
    advance(400);
    assert.ok(Math.abs(element.currentTime - before - 0.4) < 1e-6);
    element.pause();
    assert.equal(element.paused, true);
    const paused = element.currentTime;
    advance(400);
    assert.equal(element.currentTime, paused);
    element.currentTime = 0.25;
    assert.equal(element.currentTime, 0.25);
    assert.ok(stub.calls.some((call) => call.name === "audio.play"));
});

test("video elements fire ended at the end of playback", async () => {
    stub.files.set("video/end.mp4", new Uint8Array([1]));
    const element = document.createElement("video");
    element.src = "video/end.mp4";
    let ended = 0;
    element.onended = () => ended++;
    const playing = element.play();
    advance(1);
    await playing;
    advance(1500);
    assert.equal(ended, 1);
    assert.equal(element.ended, true);
    assert.equal(element.paused, true);
    assert.equal(element.currentTime, 1);
});

test("video elements play their audio through the bundle", async () => {
    stub.files.set("video/battle.mp4", new Uint8Array([1, 2, 3, 4]));
    const element = document.createElement("video");
    element.src = "video/battle.mp4";
    element.volume = 0.5;
    const playing = element.play();
    advance(1);
    await playing;
    const decode = stub.calls.filter((call) => call.name === "audio.decodeAsset").at(-1);
    assert.equal(decode.args[1], "video/battle.mp4");
    const play = stub.calls.filter((call) => call.name === "audio.play").at(-1);
    assert.ok(Math.abs(play.args[2] - 0.5) < 1e-6);
});

test("video sources outside the bundle decode from fetched bytes", async () => {
    const element = document.createElement("video");
    element.src = "data:video/mp4;base64,AQIDBA==";
    const playing = element.play();
    advance(1);
    await playing;
    const opened = stub.calls.filter((call) => call.name === "video.open").at(-1);
    assert.deepEqual(opened.args, [4]);
    const decode = stub.calls.filter((call) => call.name === "audio.decode").at(-1);
    assert.deepEqual(decode.args.slice(1), [4]);
});

test("video readiness tracks decoded frames and resolves seeks", async () => {
    stub.files.set("video/ready.mp4", new Uint8Array([1, 2]));
    const element = document.createElement("video");
    element.src = "video/ready.mp4";
    assert.equal(element.readyState, 0);
    assert.equal(element.networkState, 1);
    const events = [];
    element.addEventListener("seeking", () => events.push("seeking"));
    element.addEventListener("seeked", () => events.push("seeked"));
    const playing = element.play();
    advance(1);
    await playing;
    assert.equal(element.readyState, 2);
    assert.equal(element.networkState, 1);

    const video = stub.videos.get(element.__nativeId);
    video.ready = false;
    element.currentTime = 0.25;
    assert.deepEqual(events, ["seeking"]);
    assert.equal(element.readyState, 1);
    video.ready = true;
    advance(100);
    assert.deepEqual(events, ["seeking", "seeked"]);
    assert.equal(element.readyState, 2);
});

test("video textures upload the frame at the current time", async () => {
    stub.files.set("video/cutscene.mp4", new Uint8Array([1, 2, 3]));
    const element = document.createElement("video");
    element.src = "video/cutscene.mp4";
    const playing = element.play();
    advance(1);
    await playing;
    advance(200);

    const canvas = makeCanvas();
    const gl = canvas.getContext("webgl");
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, element);

    const call = stub.calls.filter((entry) => entry.name === "gl.texImage2DVideo").at(-1);
    assert.ok(call);
    assert.equal(call.args[4], element.__nativeId);
    assert.ok(Math.abs(call.args[5] - element.currentTime) < 1e-6);
});

test("changing the video source closes the native handle", async () => {
    stub.files.set("video/first.mp4", new Uint8Array([1]));
    stub.files.set("video/second.mp4", new Uint8Array([2]));
    const element = document.createElement("video");
    element.src = "video/first.mp4";
    const playing = element.play();
    advance(1);
    await playing;
    const id = element.__nativeId;
    element.src = "video/second.mp4";
    const close = stub.calls.filter((entry) => entry.name === "video.close").at(-1);
    assert.deepEqual(close.args, [id]);
    assert.equal(element.__nativeId, 0);
    assert.equal(element.videoWidth, 0);
    assert.equal(element.paused, true);
});

test("video elements report which container they can play", () => {
    const element = document.createElement("video");
    assert.equal(element.canPlayType("video/mp4; codecs=\"avc1.42E01E\""), "probably");
    assert.equal(element.canPlayType("video/webm"), "");
});

test("suspending and resuming an audio context pauses the engine", async () => {
    const context = new AudioContext();
    await context.suspend();
    assert.equal(context.state, "suspended");
    assert.deepEqual(stub.calls.filter((call) => call.name === "audio.setPaused").at(-1).args, [true]);
    await context.resume();
    assert.equal(context.state, "running");
    assert.deepEqual(stub.calls.filter((call) => call.name === "audio.setPaused").at(-1).args, [false]);
});

test("web build HTML creates its nested canvas and startup elements", () => {
    globalThis.__kapNative_loadDocument(`<!doctype html><html><head>
        <title>Native &amp; Web</title><link rel="modulepreload" href="./assets/shared.js">
        <script type="module" src="./assets/main.js">globalThis.inlineRan = true;</script>
        <style>.game { color: red }</style>
        </head><body class="game"><!-- <canvas id="wrong"></canvas> -->
        <div class="viewport" id="viewport"><canvas id="game-canvas" width="480" height="270"></canvas></div>
        </body></html>`);
    const canvas = document.querySelector("canvas");
    assert.ok(canvas instanceof HTMLCanvasElement);
    assert.equal(canvas.width, 480);
    assert.equal(canvas.height, 270);
    assert.equal(document.getElementById("game-canvas"), canvas);
    assert.equal(canvas.parentElement, document.querySelector(".viewport"));
    assert.equal(canvas.parentElement.parentElement, document.body);
    assert.equal(document.title, "Native & Web");
    assert.equal(document.getElementById("wrong"), null);
    assert.equal(globalThis.inlineRan, undefined);
    const links = document.querySelectorAll('link[rel="modulepreload"]');
    assert.equal(links.length, 1);
    assert.equal(links[0].href, "./assets/shared.js");
    assert.equal(document.createElement("link").relList.supports("modulepreload"), true);
    document.body.classList.add("hide-cursor");
    assert.equal(document.body.classList.contains("game"), true);
    assert.equal(document.body.classList.toggle("hide-cursor"), false);
    globalThis.__kapNative_loadDocument("<body><canvas id='replacement'></canvas></body>");
    assert.equal(document.getElementById("game-canvas"), null);
    assert.equal(document.querySelector("canvas").id, "replacement");
});

test("media capability checks report video container support", async () => {
    const mp4 = await navigator.mediaCapabilities.decodingInfo({ type: "file", video: { contentType: "video/mp4" } });
    assert.equal(mp4.supported, true);
    assert.equal(mp4.smooth, false);
    const webm = await navigator.mediaCapabilities.decodingInfo({ type: "file", video: { contentType: "video/webm" } });
    assert.equal(webm.supported, false);
});

test("scheduled oscillators and biquads produce filtered PCM and free it on end", () => {
    const context = new AudioContext();
    function note(frequency) {
        const oscillator = context.createOscillator();
        oscillator.frequency.setValueAtTime(frequency, context.currentTime);
        const filter = context.createBiquadFilter();
        filter.type = "bandpass";
        filter.frequency.value = 1000;
        filter.Q.value = 5;
        const gain = context.createGain();
        gain.gain.setValueAtTime(0.1, context.currentTime);
        gain.gain.linearRampToValueAtTime(0, context.currentTime + 0.03);
        oscillator.connect(filter).connect(gain).connect(context.destination);
        oscillator.start(context.currentTime);
        oscillator.stop(context.currentTime + 0.03);
        advance(0);
        const play = stub.calls.findLast((call) => call.name === "audio.play");
        const buffer = stub.audio.buffers.get(play.args[1]);
        assert.ok(buffer.data.every(Number.isFinite));
        assert.ok(buffer.frames >= 1439 && buffer.frames <= 1441);
        const energy = buffer.data.reduce((sum, sample) => sum + sample * sample, 0);
        stub.audio.end(play.args[0]);
        assert.equal(stub.audio.buffers.has(play.args[1]), false);
        return energy;
    }
    const pass = note(1000);
    const rejected = note(8000);
    assert.ok(pass > 0.01);
    assert.ok(rejected < pass / 10, `${pass} vs ${rejected}`);
});

test("styles expose the CSS property API", () => {
    const element = document.createElement("div");
    element.style.setProperty("opacity", "0.5");
    element.style.setProperty("z-index", "-999");
    assert.equal(element.style.opacity, "0.5");
    assert.equal(element.style.zIndex, "-999");
    assert.equal(element.style.getPropertyValue("z-index"), "-999");
    assert.equal(element.style.getPropertyValue("color"), "");
    assert.equal(element.style.cssText.includes("opacity: 0.5"), true);
    assert.equal(element.style.cssText.includes("z-index: -999"), true);

    assert.equal(element.style.removeProperty("z-index"), "-999");
    assert.equal(element.style.getPropertyValue("z-index"), "");
    assert.equal(element.style.cssText.includes("z-index"), false);

    element.style.setProperty("--accent", "#fff");
    assert.equal(element.style.getPropertyValue("--accent"), "#fff");
    assert.equal(element.style.cssText.includes("--accent: #fff"), true);
});

test("assigning a css string to element.style parses it like cssText", () => {
    const element = document.createElement("input");
    element.style.width = "320px";
    element.style = "opacity: 0; z-index: -999";

    assert.equal(typeof element.style.setProperty, "function");
    assert.equal(element.style.opacity, "0");
    assert.equal(element.style.getPropertyValue("z-index"), "-999");
    assert.equal(element.style.width, undefined);

    element.style.setProperty("opacity", "1");
    assert.equal(element.style.opacity, "1");
    assert.equal(element.style.cssText.includes("opacity: 1"), true);
});

test("focused text controls receive typing instead of the canvas", () => {
    const canvas = makeCanvas();
    const canvasKeys = [];
    canvas.addEventListener("keyup", (event) => canvasKeys.push(event.key));

    const input = document.createElement("input");
    input.maxLength = 2;
    document.body.appendChild(input);
    input.focus();

    const seen = [];
    const values = [];
    input.addEventListener("keydown", (event) => seen.push(`down:${event.key}`));
    input.addEventListener("keyup", (event) => seen.push(`up:${event.key}`));
    input.addEventListener("input", () => values.push(input.value));

    globalThis.__kapNative_dispatch("keydown", JSON.stringify({ key: "a", code: "KeyA", repeat: false }));
    globalThis.__kapNative_dispatch("keyup", JSON.stringify({ key: "a", code: "KeyA", repeat: false }));
    globalThis.__kapNative_dispatch("keydown", JSON.stringify({ key: "b", code: "KeyB", repeat: false }));
    globalThis.__kapNative_dispatch("keydown", JSON.stringify({ key: "c", code: "KeyC", repeat: false }));
    globalThis.__kapNative_dispatch("keydown", JSON.stringify({ key: "Backspace", code: "Backspace", repeat: false }));

    assert.equal(input.value, "a");
    assert.deepEqual(values, ["a", "ab", "a"]);
    assert.deepEqual(seen, ["down:a", "up:a", "down:b", "down:c", "down:Backspace"]);
    assert.deepEqual(canvasKeys, []);

    input.blur();
    globalThis.__kapNative_dispatch("keyup", JSON.stringify({ key: "q", code: "KeyQ", repeat: false }));
    assert.deepEqual(canvasKeys, ["q"]);
    input.remove();
});

test("Enter adds a line break to a focused textarea only", () => {
    const inputValues = [];
    const input = document.createElement("input");
    input.addEventListener("input", () => inputValues.push(input.value));
    document.body.appendChild(input);
    input.focus();
    globalThis.__kapNative_dispatch("keydown", JSON.stringify({ key: "x", code: "KeyX", repeat: false }));
    globalThis.__kapNative_dispatch("keydown", JSON.stringify({ key: "Enter", code: "Enter", repeat: false }));
    assert.equal(input.value, "x");

    const textareaValues = [];
    const textarea = document.createElement("textarea");
    textarea.addEventListener("input", () => textareaValues.push(textarea.value));
    document.body.appendChild(textarea);
    textarea.focus();
    globalThis.__kapNative_dispatch("keydown", JSON.stringify({ key: "y", code: "KeyY", repeat: false }));
    globalThis.__kapNative_dispatch("keydown", JSON.stringify({ key: "Enter", code: "Enter", repeat: false }));
    assert.equal(textarea.value, "y\n");
    assert.deepEqual(inputValues, ["x"]);
    assert.deepEqual(textareaValues, ["y", "y\n"]);

    input.remove();
    textarea.remove();
});

test("removing or clicking away from a focused input releases the keys", () => {
    const canvas = makeCanvas();
    const canvasKeys = [];
    canvas.addEventListener("keydown", (event) => canvasKeys.push(event.key));

    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();
    assert.equal(document.activeElement, input);

    globalThis.__kapNative_dispatch("mousedown", JSON.stringify({ clientX: 5, clientY: 5, button: 0 }));
    globalThis.__kapNative_dispatch("mouseup", JSON.stringify({ clientX: 5, clientY: 5, button: 0 }));
    assert.equal(document.activeElement, null);
    globalThis.__kapNative_dispatch("keydown", JSON.stringify({ key: "w", code: "KeyW", repeat: false }));
    assert.deepEqual(canvasKeys, ["w"]);

    input.focus();
    input.remove();
    assert.equal(document.activeElement, null);

    const wrapper = document.createElement("div");
    const nested = document.createElement("input");
    wrapper.appendChild(nested);
    document.body.appendChild(wrapper);
    nested.focus();
    wrapper.remove();
    assert.equal(document.activeElement, null);
});

test("clicking a positioned text control focuses it instead of the canvas", () => {
    const canvas = makeCanvas();
    const canvasPresses = [];
    canvas.addEventListener("mousedown", () => canvasPresses.push("canvas"));

    const input = document.createElement("input");
    input.maxLength = 16;
    document.body.appendChild(input);
    input.style = "position:absolute;left:40px;top:40px;width:120px;height:30px;opacity:0";
    input.addEventListener("click", () => input.focus());
    input.click();
    assert.equal(document.activeElement, input);

    globalThis.__kapNative_dispatch("mousedown", JSON.stringify({ clientX: 60, clientY: 50, button: 0 }));
    globalThis.__kapNative_dispatch("mouseup", JSON.stringify({ clientX: 60, clientY: 50, button: 0 }));
    assert.equal(document.activeElement, input);
    assert.deepEqual(canvasPresses, []);

    globalThis.__kapNative_dispatch("keydown", JSON.stringify({ key: "a", code: "KeyA", repeat: false }));
    assert.equal(input.value, "a");

    globalThis.__kapNative_dispatch("mousedown", JSON.stringify({ clientX: 300, clientY: 200, button: 0 }));
    globalThis.__kapNative_dispatch("mouseup", JSON.stringify({ clientX: 300, clientY: 200, button: 0 }));
    assert.equal(document.activeElement, null);
    assert.deepEqual(canvasPresses, ["canvas"]);
    input.remove();
});

test("text controls behind the canvas keep clicks on the canvas", () => {
    const canvas = makeCanvas();
    const canvasPresses = [];
    canvas.addEventListener("mousedown", () => canvasPresses.push("canvas"));

    const input = document.createElement("input");
    document.body.appendChild(input);
    input.style = "position:absolute;left:10px;top:10px;width:80px;height:20px;z-index:-999";

    globalThis.__kapNative_dispatch("mousedown", JSON.stringify({ clientX: 20, clientY: 15, button: 0 }));
    globalThis.__kapNative_dispatch("mouseup", JSON.stringify({ clientX: 20, clientY: 15, button: 0 }));
    assert.equal(canvasPresses.length, 1);
    assert.notEqual(document.activeElement, input);
    input.remove();
});

test("screen is exposed globally with the window size", () => {
    assert.equal(typeof screen.width, "number");
    assert.equal(typeof screen.height, "number");

    globalThis.__kapNative_dispatch(
        "resize",
        JSON.stringify({ width: 800, height: 600, dpr: 2 }),
    );
    assert.equal(screen.width, 800);
    assert.equal(screen.height, 600);
    assert.equal(screen.dpr, 2);
});

test("window.open hands the url to the system browser and resolves the auth callback", () => {
    const messages = [];
    const listener = (event) => messages.push(event);
    window.addEventListener("message", listener);

    const popup = window.open(
        "https://game.example/auth/provider/discord?token=abc",
        "Login with Discord",
        "popup,width=480,height=768",
    );
    assert.equal(typeof popup.close, "function");
    assert.equal(popup.closed, false);

    const opened = stub.calls.filter((call) => call.name === "auth.open").at(-1);
    assert.deepEqual(opened.args, ["https://game.example/auth/provider/discord?token=abc"]);

    assert.equal(window.open("https://game.example/auth/provider/discord"), null);
    assert.equal(window.open("file:///etc/passwd"), null);

    globalThis.__kapNative_onAuthCallback('{"user":{"id":"1"},"token":"jwt"}');
    assert.equal(popup.closed, true);
    assert.equal(messages.length, 1);
    assert.equal(messages[0].type, "message");
    assert.deepEqual(messages[0].data, { user: { id: "1" }, token: "jwt" });

    globalThis.__kapNative_onAuthCallback("");
    assert.equal(messages.length, 1);
    window.removeEventListener("message", listener);
});
