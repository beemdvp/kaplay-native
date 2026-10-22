export function installStub() {
    const calls = [];
    const logs = [];
    const files = new Map([["font.ttf", new Uint8Array([1, 2, 3, 4])]]);
    const images = new Map();
    const canvases = new Map();
    const videos = new Map();

    function finishDecode(request) {
        const id = nextId();
        audioBuffers.set(id, { sampleRate: 48000, channels: 2, frames: 4800, data: null });
        globalThis.__kapNative_onAudioDecoded(request, id, true, "");
    }

    function openVideo() {
        const id = nextId();
        videos.set(id, { width: 160, height: 120, duration: 1, ready: true });
        return { id, width: 160, height: 120, duration: 1 };
    }
    const netRoutes = new Map();
    const netResponses = new Map();
    const netMessages = new Map();
    const audioBuffers = new Map();
    const audioVoices = new Map();
    const openSockets = [];
    const state = {
        time: 0,
        storage: new Map(),
        gamepads: [],
        analyser: [],
        nextId: 1,
        nextCanvas: 1,
    };

    function record(name, args) {
        calls.push({ name, args });
    }

    function nextId() {
        return state.nextId++;
    }

    globalThis.__kapNative = {
        env: {
            hostVersion: "test",
            platform: "test",
            gameName: "stub",
            networkAllowed: false,
            startUrl: "kaplay-native://game/",
            now: () => state.time,
            log: (level, message) => logs.push({ level, message }),
            quit: () => record("env.quit", []),
        },
        window: {
            size: () => [960, 540],
            setTitle: (title) => record("window.setTitle", [title]),
            setFullscreen: (value) => record("window.setFullscreen", [value]),
            isFullscreen: () => false,
            setCursor: (value) => record("window.setCursor", [value]),
            setCursorLocked: (value) => record("window.setCursorLocked", [value]),
            isCursorLocked: () => false,
        },
        fs: {
            size: (path) => {
                const data = files.get(path);
                return data ? data.length : -1;
            },
            exists: (path) => files.has(path),
            read: (path, out) => {
                const data = files.get(path);
                if (!data) return false;
                out.set(data.subarray(0, out.length));
                return true;
            },
            download: (name, data) => {
                record("fs.download", [String(name), Array.from(data)]);
                return "/downloads/" + name;
            },
        },
        storage: {
            load: () => JSON.stringify(Object.fromEntries(state.storage)),
            save: (text) => {
                state.storage = new Map(Object.entries(JSON.parse(text)));
                record("storage.save", [text]);
            },
        },
        image: {
            load: (url) => {
                if (files.has(url)) {
                    const id = nextId();
                    images.set(id, { width: 4, height: 3 });
                    return id;
                }
                return -1;
            },
            decode: (bytes) => {
                const id = nextId();
                images.set(id, { width: bytes.length, height: 1 });
                return id;
            },
            width: (id) => images.get(id)?.width ?? -1,
            height: (id) => images.get(id)?.height ?? -1,
            free: (id) => {
                images.delete(id);
                record("image.free", [id]);
            },
        },
        font: {
            register: (family, bytes) => {
                record("font.register", [family, bytes.length]);
                return true;
            },
            has: (family) => family === "monospace" || family === "MyFont",
        },
        canvas: {
            create: (width, height) => {
                const id = nextId();
                canvases.set(id, { width, height });
                record("canvas.create", [id, width, height]);
                return id;
            },
            resize: (id, width, height) => {
                const canvas = canvases.get(id);
                if (canvas) {
                    canvas.width = width;
                    canvas.height = height;
                }
                record("canvas.resize", [id, width, height]);
            },
            destroy: (id) => record("canvas.destroy", [id]),
            createContext: (id) => nextId(),
            setFillStyle: (context, value) => record("canvas.setFillStyle", [context, value]),
            setStrokeStyle: (context, value) => record("canvas.setStrokeStyle", [context, value]),
            setFont: (context, value) => record("canvas.setFont", [context, value]),
            setLineWidth: (context, value) => record("canvas.setLineWidth", [context, value]),
            setLineJoin: (context, value) => record("canvas.setLineJoin", [context, value]),
            setTextBaseline: (context, value) => record("canvas.setTextBaseline", [context, value]),
            setTextAlign: (context, value) => record("canvas.setTextAlign", [context, value]),
            clearRect: (...args) => record("canvas.clearRect", args),
            fillText: (...args) => record("canvas.fillText", args),
            strokeText: (...args) => record("canvas.strokeText", args),
            measureText: (context, text) => {
                record("canvas.measureText", [context, text]);
                return [text.length * 7, 10, 2];
            },
            drawImage: (...args) => record("canvas.drawImage", args),
            putImageData: (...args) => record("canvas.putImageData", args),
            getImageData: (context, x, y, width, height, out) => {
                record("canvas.getImageData", [context, x, y, width, height]);
                for (let i = 0; i < out.length; i++) out[i] = i % 251;
            },
            pngLength: () => 4,
            copyPng: (canvas, out) => {
                out.set([1, 2, 3, 4]);
                return true;
            },
        },
        gl: {},
        gamepad: {
            list: () => {
                const value = JSON.stringify(state.gamepads);
                record("gamepad.list", [value]);
                return value;
            },
            revision: () => JSON.stringify(state.gamepads),
        },
        auth: {
            open: (url) => {
                record("auth.open", [url]);
                return true;
            },
        },
        net: {
            fetch: (id, method, url, headers, body) => {
                record("net.fetch", [id, method, url, JSON.parse(headers), body ? Array.from(body) : null]);
                const route = netRoutes.get(url);
                if (!route) {
                    globalThis.__kapNative_onFetchError(id, "connection refused");
                    return true;
                }
                netResponses.set(id, route);
                if (route.defer) return true;
                const status = route.status === undefined ? 200 : route.status;
                globalThis.__kapNative_onFetch(
                    id,
                    url,
                    status,
                    route.statusText === undefined ? "OK" : route.statusText,
                    JSON.stringify(route.headers || []),
                    route.body ? route.body.length : 0,
                    status >= 200 && status < 300,
                );
                return true;
            },
            openSocket: (id, url) => {
                record("net.openSocket", [id, url]);
                openSockets.push(id);
                return true;
            },
            sendText: (id, text) => record("net.sendText", [id, text]),
            sendBinary: (id, data) => record("net.sendBinary", [id, Array.from(data)]),
            closeSocket: (id, code, reason) => record("net.closeSocket", [id, code, reason]),
            readResponse: (id, out) => {
                const route = netResponses.get(id);
                if (!route) return -1;
                netResponses.delete(id);
                const body = route.body || new Uint8Array(0);
                out.set(body.subarray(0, out.length));
                return Math.min(body.length, out.length);
            },
            readMessage: (id, out) => {
                const queue = netMessages.get(id);
                if (!queue || queue.length === 0) return -1;
                const data = queue.shift();
                out.set(data.subarray(0, out.length));
                return data.length;
            },
        },
        video: {
            open: (bytes) => {
                record("video.open", [bytes.length]);
                return openVideo();
            },
            openAsset: (path) => {
                record("video.openAsset", [String(path)]);
                return openVideo();
            },
            close: (id) => record("video.close", [id]),
            ready: (id) => {
                const video = videos.get(id);
                return Boolean(video && video.ready);
            },
        },
        audio: {
            available: () => true,
            sampleRate: () => 48000,
            currentTime: () => state.time / 1000,
            setPaused: (paused) => record("audio.setPaused", [paused]),
            decode: (request, bytes) => {
                record("audio.decode", [request, bytes.length]);
                finishDecode(request);
            },
            decodeAsset: (request, path) => {
                record("audio.decodeAsset", [request, String(path)]);
                finishDecode(request);
            },
            uploadBuffer: (sampleRate, channels, data) => {
                const id = nextId();
                audioBuffers.set(id, {
                    sampleRate,
                    channels,
                    frames: Math.floor(data.length / channels),
                    data: Float32Array.from(data),
                });
                record("audio.uploadBuffer", [id, sampleRate, channels, data.length]);
                return id;
            },
            bufferInfo: (id, out) => {
                const buffer = audioBuffers.get(id);
                if (!buffer) return false;
                out[0] = buffer.sampleRate;
                out[1] = buffer.channels;
                out[2] = buffer.frames;
                return true;
            },
            analyser: (out) => {
                out.set(state.analyser.slice(Math.max(0, state.analyser.length - out.length)));
                return out.length;
            },
            readChannel: (id, channel, out) => {
                const buffer = audioBuffers.get(id);
                if (!buffer || !buffer.data) {
                    out.fill(0);
                    return out.length;
                }
                const frames = Math.floor(buffer.data.length / buffer.channels);
                for (let i = 0; i < out.length && i < frames; i++) {
                    out[i] = buffer.data[i * buffer.channels + channel] || 0;
                }
                return out.length;
            },
            freeBuffer: (id) => {
                audioBuffers.delete(id);
                record("audio.freeBuffer", [id]);
            },
            play: (buffer, gain, pan, rate, looping, offset, startAt) => {
                const id = nextId();
                audioVoices.set(id, {
                    buffer,
                    gain,
                    pan,
                    rate,
                    looping,
                    offset,
                    startAt,
                    time: state.time / 1000,
                });
                record("audio.play", [id, buffer, gain, pan, rate, looping, offset, startAt]);
                return id;
            },
            setGain: (voice, gain) => {
                const entry = audioVoices.get(voice);
                if (entry) entry.gain = gain;
                record("audio.setGain", [voice, gain]);
            },
            setPan: (voice, pan) => {
                const entry = audioVoices.get(voice);
                if (entry) entry.pan = pan;
                record("audio.setPan", [voice, pan]);
            },
            setRate: (voice, rate) => {
                const entry = audioVoices.get(voice);
                if (entry) entry.rate = rate;
                record("audio.setRate", [voice, rate]);
            },
            setLoop: (voice, looping) => {
                const entry = audioVoices.get(voice);
                if (entry) entry.looping = looping;
                record("audio.setLoop", [voice, looping]);
            },
            stop: (voice) => {
                audioVoices.delete(voice);
                record("audio.stop", [voice]);
            },
            stopAt: (voice, frame) => {
                const entry = audioVoices.get(voice);
                if (entry) entry.stopAt = frame;
                record("audio.stopAt", [voice, frame]);
            },
            position: (voice) => {
                const entry = audioVoices.get(voice);
                if (!entry) return -1;
                return state.time / 1000 - entry.time + entry.offset;
            },
            duration: (voice) => {
                const entry = audioVoices.get(voice);
                const buffer = entry && audioBuffers.get(entry.buffer);
                return buffer ? buffer.frames / buffer.sampleRate : -1;
            },
        },
        updates: {
            snapshot: () =>
                JSON.stringify({
                    status: "ready",
                    current: "1.0.0",
                    version: "1.1.0",
                    required: true,
                    progress: 100,
                }),
            apply: () => {
                record("updates.apply", []);
                return true;
            },
        },
        steam: {
            available: () => true,
            playerName: () => "Tester",
            steamId: () => "76561198000000000",
            unlockAchievement: (name) => {
                record("steam.unlockAchievement", [name]);
                return name !== "locked";
            },
            isAchievementUnlocked: (name) => {
                record("steam.isAchievementUnlocked", [name]);
                return name === "unlocked";
            },
            storeStats: () => {
                record("steam.storeStats", []);
                return true;
            },
            setStatInt: (name, value) => {
                record("steam.setStatInt", [name, value]);
                return true;
            },
            getStatInt: (name) => {
                record("steam.getStatInt", [name]);
                return name === "missing" ? null : 7;
            },
            setStatFloat: (name, value) => {
                record("steam.setStatFloat", [name, value]);
                return true;
            },
            getStatFloat: (name) => {
                record("steam.getStatFloat", [name]);
                return name === "missing" ? null : 2.5;
            },
            writeCloud: (name, text) => {
                record("steam.writeCloud", [name, text]);
                return true;
            },
            readCloud: (name) => {
                record("steam.readCloud", [name]);
                return name === "missing" ? null : "cloud-data";
            },
            cloudExists: (name) => {
                record("steam.cloudExists", [name]);
                return name !== "missing";
            },
            deleteCloud: (name) => {
                record("steam.deleteCloud", [name]);
                return true;
            },
            runCallbacks: () => record("steam.runCallbacks", []),
        },
    };

    const glCalls = [
        "viewport",
        "clearColor",
        "clear",
        "enable",
        "disable",
        "blendFuncSeparate",
        "stencilFunc",
        "stencilOp",
        "pixelStorei",
        "getParameter",
        "activeTexture",
        "createTexture",
        "deleteTexture",
        "bindTexture",
        "texParameteri",
        "texImage2D",
        "texSubImage2D",
        "texImage2DImage",
        "texSubImage2DImage",
        "texImage2DCanvas",
        "texSubImage2DCanvas",
        "createBuffer",
        "deleteBuffer",
        "bindBuffer",
        "bufferDataSize",
        "bufferData",
        "bufferSubData",
        "enableVertexAttribArray",
        "disableVertexAttribArray",
        "vertexAttribPointer",
        "drawArrays",
        "drawElements",
        "createShader",
        "deleteShader",
        "shaderSource",
        "compileShader",
        "getShaderInfoLog",
        "createProgram",
        "deleteProgram",
        "attachShader",
        "bindAttribLocation",
        "linkProgram",
        "getLinkStatus",
        "getProgramInfoLog",
        "useProgram",
        "getAttribLocation",
        "getUniformLocation",
        "uniform1f",
        "uniform1i",
        "uniform2f",
        "uniform3f",
        "uniform1fv",
        "uniform2fv",
        "uniform3fv",
        "uniformMatrix4fv",
        "createFramebuffer",
        "deleteFramebuffer",
        "bindFramebuffer",
        "createRenderbuffer",
        "deleteRenderbuffer",
        "bindRenderbuffer",
        "renderbufferStorage",
        "framebufferTexture2D",
        "framebufferRenderbuffer",
        "checkFramebufferStatus",
        "readPixels",
    ];
    for (const name of glCalls) {
        globalThis.__kapNative.gl[name] = (...args) => {
            record(`gl.${name}`, args);
            if (name === "createTexture" || name === "createBuffer") return nextId();
            if (name === "createShader" || name === "createProgram") return nextId();
            if (name === "createFramebuffer" || name === "createRenderbuffer") return nextId();
            if (name === "getShaderInfoLog" || name === "getProgramInfoLog") return "";
            if (name === "getLinkStatus") return true;
            if (name === "getUniformLocation") {
                return String(args[1]).startsWith("missing") ? -1 : nextId();
            }
            if (name === "getAttribLocation") return 0;
            if (name === "getParameter") return 16;
            if (name === "checkFramebufferStatus") return 0x8cd5;
            return undefined;
        };
    }
    globalThis.__kapNative.gl.drawingBufferWidth = () => 320;
    globalThis.__kapNative.gl.drawingBufferHeight = () => 240;
    globalThis.__kapNative.gl.resizeDrawingBuffer = (...args) => record("gl.resizeDrawingBuffer", args);
    globalThis.__kapNative.gl.screenshotLength = () => {
        record("gl.screenshotLength", []);
        return 4;
    };
    globalThis.__kapNative.gl.copyScreenshot = (out) => {
        record("gl.copyScreenshot", [out.length]);
        out.set([9, 8, 7, 6]);
        return true;
    };
    globalThis.__kapNative.gl.texImage2DVideo = (...args) => record("gl.texImage2DVideo", args);
    globalThis.__kapNative.glCalls = () => globalThis.__kapNative.gl;

    const net = {
        routes: netRoutes,
        opened: openSockets,
        route(url, options) {
            const route = options || {};
            const body = route.body === undefined
                ? new Uint8Array(0)
                : typeof route.body === "string"
                    ? new TextEncoder().encode(route.body)
                    : route.body;
            netRoutes.set(url, {
                status: route.status,
                statusText: route.statusText,
                headers: route.headers,
                body,
                defer: Boolean(route.defer),
            });
        },
        open(id) {
            globalThis.__kapNative_onSocketOpen(id);
        },
        deliver(id, data, text) {
            const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data;
            const isText = text === undefined ? typeof data === "string" : Boolean(text);
            const queue = netMessages.get(id) || [];
            queue.push(bytes);
            netMessages.set(id, queue);
            globalThis.__kapNative_onSocketMessage(id, bytes.length, isText);
        },
        close(id, code, reason) {
            globalThis.__kapNative_onSocketClose(id, code === undefined ? 1000 : code, reason || "");
        },
        fail(id, message) {
            globalThis.__kapNative_onSocketError(id, message);
        },
    };

    const audio = {
        buffers: audioBuffers,
        voices: audioVoices,
        end(voice) {
            globalThis.__kapNative_onVoiceEnded(voice);
        },
    };

    return { calls, logs, state, files, canvases, videos, net, audio };
}
