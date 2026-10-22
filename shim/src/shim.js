(function () {
    "use strict";

    const native = __kapNative;
    const global = globalThis;

    const screen = {
        width: 960,
        height: 540,
        dpr: 1,
    };

    try {
        const size = native.window.size();
        screen.width = size[0];
        screen.height = size[1];
    } catch (error) {
        reportError(error);
    }

    function reportError(error) {
        const message = error && error.stack ? error.stack : String(error);
        native.env.log("error", message);
    }

    const releaseFinalizer = typeof FinalizationRegistry === "function"
        ? new FinalizationRegistry((release) => {
            try {
                release();
            } catch (error) {
                reportError(error);
            }
        })
        : null;

    function registerRelease(target, release) {
        if (!releaseFinalizer) return;
        const token = {};
        target.__releaseToken = token;
        releaseFinalizer.register(target, release, token);
    }

    function unregisterRelease(target) {
        if (!releaseFinalizer) return;
        const token = target.__releaseToken;
        if (!token) return;
        releaseFinalizer.unregister(token);
        target.__releaseToken = null;
    }

    function bytesView(value) {
        if (value instanceof Uint8Array) return value;
        if (value instanceof Uint8ClampedArray) {
            return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
        }
        if (ArrayBuffer.isView(value)) {
            return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
        }
        if (value instanceof ArrayBuffer) return new Uint8Array(value);
        return new Uint8Array(0);
    }

    function float32(value) {
        if (value instanceof Float32Array) return value;
        if (value && typeof value.length === "number") return new Float32Array(value);
        return new Float32Array(0);
    }

    // ---------------------------------------------------------------- listeners

    function makeListeners() {
        const map = new Map();
        return {
            add(type, fn) {
                if (typeof fn !== "function") return;
                const list = map.get(type);
                if (list) list.push(fn);
                else map.set(type, [fn]);
            },
            remove(type, fn) {
                const list = map.get(type);
                if (!list) return;
                const index = list.indexOf(fn);
                if (index >= 0) list.splice(index, 1);
            },
            emit(event) {
                const list = map.get(event.type);
                if (list) {
                    for (const fn of list.slice()) {
                        try {
                            fn.call(null, event);
                        } catch (error) {
                            reportError(error);
                        }
                    }
                }
            },
        };
    }

    const canvasListeners = makeListeners();
    const documentListeners = makeListeners();
    const windowListeners = makeListeners();

    // ------------------------------------------------------------------ timers

    const timers = new Map();
    let frames = [];
    let nextTimer = 1;
    let nextFrame = 1;
    let frameTime = 0;
    let inFrame = false;
    const dueTimers = [];

    function timerNow() {
        return inFrame ? frameTime : native.env.now();
    }

    const noArgs = [];

    function setTimeoutFn(fn, delay) {
        const args = arguments.length > 2 ? Array.prototype.slice.call(arguments, 2) : noArgs;
        const id = nextTimer++;
        timers.set(id, { fn, args, due: timerNow() + Number(delay || 0), interval: 0 });
        return id;
    }

    function setIntervalFn(fn, delay) {
        const args = arguments.length > 2 ? Array.prototype.slice.call(arguments, 2) : noArgs;
        const id = nextTimer++;
        const interval = Math.max(1, Number(delay || 0));
        timers.set(id, { fn, args, due: timerNow() + interval, interval });
        return id;
    }

    function clearTimer(id) {
        timers.delete(id);
    }

    function requestAnimationFrame(callback) {
        const id = nextFrame++;
        frames.push({ id, callback });
        return id;
    }

    function cancelAnimationFrame(id) {
        const index = frames.findIndex((entry) => entry.id === id);
        if (index >= 0) frames.splice(index, 1);
    }

    function tickTimers(now) {
        if (timers.size === 0) return;
        const due = dueTimers;
        due.length = 0;
        let ordered = true;
        let previousDue = -Infinity;
        for (const entry of timers) {
            const timer = entry[1];
            if (timer.due <= now) {
                if (timer.due < previousDue) ordered = false;
                previousDue = timer.due;
                due.push(entry);
            }
        }
        if (!ordered) due.sort((a, b) => a[1].due - b[1].due || a[0] - b[0]);
        for (const [id, timer] of due) {
            if (!timers.has(id)) continue;
            if (timer.interval > 0) timer.due = now + timer.interval;
            else timers.delete(id);
            try {
                timer.fn.apply(null, timer.args);
            } catch (error) {
                reportError(error);
            }
        }
        due.length = 0;
    }

    // ------------------------------------------------------------------ base64

    const BASE64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    function atob(value) {
        let input = String(value).replace(/[^A-Za-z0-9+/=]/g, "");
        let output = "";
        for (let i = 0; i < input.length; i += 4) {
            const a = BASE64.indexOf(input[i]);
            const b = BASE64.indexOf(input[i + 1]);
            const c = BASE64.indexOf(input[i + 2]);
            const d = BASE64.indexOf(input[i + 3]);
            output += String.fromCharCode((a << 2) | (b >> 4));
            if (c >= 0) output += String.fromCharCode(((b & 15) << 4) | (c >> 2));
            if (d >= 0) output += String.fromCharCode(((c & 3) << 6) | d);
        }
        return output;
    }

    function btoa(value) {
        const input = String(value);
        let output = "";
        for (let i = 0; i < input.length; i += 3) {
            const a = input.charCodeAt(i) & 0xff;
            const b = i + 1 < input.length ? input.charCodeAt(i + 1) & 0xff : NaN;
            const c = i + 2 < input.length ? input.charCodeAt(i + 2) & 0xff : NaN;
            output += BASE64[a >> 2];
            output += BASE64[((a & 3) << 4) | (isNaN(b) ? 0 : b >> 4)];
            output += isNaN(b) ? "=" : BASE64[((b & 15) << 2) | (isNaN(c) ? 0 : c >> 6)];
            output += isNaN(c) ? "=" : BASE64[c & 63];
        }
        return output;
    }

    function decodeDataURL(url) {
        const match = /^data:([^,]*),([\s\S]*)$/.exec(String(url));
        if (!match) return null;
        const meta = match[1];
        const payload = match[2];
        if (/;\s*base64(?:;|$)/i.test(meta)) {
            const binary = atob(payload);
            const bytes = new Uint8Array(binary.length);
            for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
            return bytes;
        }
        try {
            return utf8Encode(decodeURIComponent(payload));
        } catch (error) {
            return utf8Encode(payload);
        }
    }

    // ------------------------------------------------------------------- style

    function stylePropertyName(name) {
        const key = String(name);
        if (key.startsWith("--")) return key;
        return key.replace(/-([a-z])/g, (_match, letter) => letter.toUpperCase());
    }

    function stylePropertyText(name) {
        const key = String(name);
        if (key.startsWith("--")) return key;
        return key.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`);
    }

    function makeStyle() {
        const style = {};
        let cursor = "";
        Object.defineProperty(style, "cursor", {
            get() {
                return cursor;
            },
            set(value) {
                cursor = String(value);
                native.window.setCursor(cursor);
            },
            enumerable: true,
            configurable: true,
        });
        style.setProperty = function (name, value) {
            style[stylePropertyName(name)] = String(value);
        };
        style.getPropertyValue = function (name) {
            const value = style[stylePropertyName(name)];
            return value === undefined || value === null ? "" : String(value);
        };
        style.removeProperty = function (name) {
            const key = stylePropertyName(name);
            const value = style.getPropertyValue(key);
            if (key === "cursor") style.cursor = "";
            else delete style[key];
            return value;
        };
        Object.defineProperty(style, "cssText", {
            get() {
                return Object.entries(style)
                    .filter(([key]) => key.startsWith("__") ? false : typeof style[key] !== "function")
                    .map(([key, value]) => `${stylePropertyText(key)}: ${value}`)
                    .join("; ");
            },
            set(value) {
                for (const key of Object.keys(style)) {
                    if (key === "cssText" || typeof style[key] === "function") continue;
                    if (key === "cursor") style.cursor = "";
                    else delete style[key];
                }
                for (const part of String(value).split(";")) {
                    const index = part.indexOf(":");
                    if (index < 0) continue;
                    const key = part.slice(0, index).trim();
                    const entry = part.slice(index + 1).trim();
                    if (key) style[stylePropertyName(key)] = entry;
                }
            },
        });
        return style;
    }

    function cssColor(value) {
        if (typeof value === "string") return value;
        if (value && typeof value.toHex === "function") return value.toHex();
        if (value && typeof value.r === "number") {
            return `rgb(${value.r}, ${value.g}, ${value.b})`;
        }
        return String(value);
    }

    // ------------------------------------------------------------------ canvas

    function canvasSize(canvas) {
        const width = canvas.offsetWidth;
        const height = canvas.offsetHeight;
        if (!width || !height) return { x: 0, y: 0, width: 0, height: 0, scale: 1 };
        const scale = Math.min(screen.width / width, screen.height / height);
        const targetWidth = width * scale;
        const targetHeight = height * scale;
        return {
            x: (screen.width - targetWidth) / 2,
            y: (screen.height - targetHeight) / 2,
            width: targetWidth,
            height: targetHeight,
            scale,
        };
    }

    function px(value) {
        if (typeof value === "string" && value.trim().endsWith("px")) {
            return parseFloat(value) || 0;
        }
        return 0;
    }

    class CanvasRenderingContext2D {
        constructor(canvas) {
            this.canvas = canvas;
            this.__id = native.canvas.createContext(canvas.__id);
            this.fillStyle = "#000000";
            this.strokeStyle = "#000000";
            this.font = "10px sans-serif";
            this.lineWidth = 1;
            this.lineJoin = "miter";
            this.textBaseline = "alphabetic";
            this.textAlign = "start";
            this.globalAlpha = 1;
            this.globalCompositeOperation = "source-over";
            this.imageSmoothingEnabled = true;
        }

        set fillStyle(value) {
            this.__fillStyle = value;
            native.canvas.setFillStyle(this.__id, cssColor(value));
        }

        get fillStyle() {
            return this.__fillStyle;
        }

        set strokeStyle(value) {
            this.__strokeStyle = value;
            native.canvas.setStrokeStyle(this.__id, cssColor(value));
        }

        get strokeStyle() {
            return this.__strokeStyle;
        }

        set font(value) {
            this.__font = value;
            native.canvas.setFont(this.__id, String(value));
        }

        get font() {
            return this.__font;
        }

        set lineWidth(value) {
            this.__lineWidth = Number(value) || 0;
            native.canvas.setLineWidth(this.__id, this.__lineWidth);
        }

        get lineWidth() {
            return this.__lineWidth;
        }

        set lineJoin(value) {
            this.__lineJoin = value;
            native.canvas.setLineJoin(this.__id, String(value));
        }

        get lineJoin() {
            return this.__lineJoin;
        }

        set textBaseline(value) {
            this.__textBaseline = value;
            native.canvas.setTextBaseline(this.__id, String(value));
        }

        get textBaseline() {
            return this.__textBaseline;
        }

        set textAlign(value) {
            this.__textAlign = value;
            native.canvas.setTextAlign(this.__id, String(value));
        }

        get textAlign() {
            return this.__textAlign;
        }

        clearRect(x, y, width, height) {
            native.canvas.clearRect(this.__id, Number(x), Number(y), Number(width), Number(height));
        }

        fillText(text, x, y) {
            native.canvas.fillText(this.__id, String(text), Number(x), Number(y));
        }

        strokeText(text, x, y) {
            native.canvas.strokeText(this.__id, String(text), Number(x), Number(y));
        }

        measureText(text) {
            const metrics = native.canvas.measureText(this.__id, String(text));
            return {
                width: metrics[0],
                actualBoundingBoxLeft: 0,
                actualBoundingBoxRight: metrics[0],
                actualBoundingBoxAscent: metrics[1],
                actualBoundingBoxDescent: metrics[2],
            };
        }

        drawImage(source, ...values) {
            let kind = 0;
            let id = -1;
            let sourceWidth = 0;
            let sourceHeight = 0;
            if (source instanceof HTMLCanvasElement) {
                kind = 1;
                id = source.__id;
                sourceWidth = source.width;
                sourceHeight = source.height;
            } else if (source instanceof ImageData) {
                this.putImageData(source, Number(values[0]) || 0, Number(values[1]) || 0);
                return;
            } else if (source instanceof Image) {
                kind = 0;
                id = source.__id;
                sourceWidth = source.width;
                sourceHeight = source.height;
            }
            if (id < 0) return;
            if (values.length >= 8) {
                native.canvas.drawImage(
                    this.__id,
                    kind,
                    id,
                    values[0],
                    values[1],
                    values[2],
                    values[3],
                    values[4],
                    values[5],
                    values[6],
                    values[7],
                );
            } else if (values.length === 4) {
                native.canvas.drawImage(
                    this.__id,
                    kind,
                    id,
                    0,
                    0,
                    sourceWidth,
                    sourceHeight,
                    values[0],
                    values[1],
                    values[2],
                    values[3],
                );
            } else {
                native.canvas.drawImage(
                    this.__id,
                    kind,
                    id,
                    0,
                    0,
                    sourceWidth,
                    sourceHeight,
                    values[0],
                    values[1],
                    sourceWidth,
                    sourceHeight,
                );
            }
        }

        getImageData(x, y, width, height) {
            const w = Math.max(0, Math.floor(Number(width) || 0));
            const h = Math.max(0, Math.floor(Number(height) || 0));
            const bytes = new Uint8Array(w * h * 4);
            native.canvas.getImageData(this.__id, Math.floor(x), Math.floor(y), w, h, bytes);
            return new ImageData(new Uint8ClampedArray(bytes.buffer), w, h);
        }

        putImageData(image, x, y) {
            native.canvas.putImageData(
                this.__id,
                Math.floor(Number(x) || 0),
                Math.floor(Number(y) || 0),
                image.width,
                image.height,
                bytesView(image.data),
            );
        }

        save() {}

        restore() {}

        translate() {}

        scale() {}

        rotate() {}

        setTransform() {}

        resetTransform() {}

        beginPath() {}

        closePath() {}

        moveTo() {}

        lineTo() {}

        stroke() {}

        fill() {}

        fillRect() {}

        strokeRect() {}

        arc() {}
    }

    class HTMLCanvasElement {
        constructor() {
            this.tagName = "CANVAS";
            this.nodeName = "CANVAS";
            this.__id = native.canvas.create(1, 1);
            this.__width = 1;
            this.__height = 1;
            this.__context2d = null;
            this.__gl = null;
            this.__parent = null;
            this.__style = makeStyle();
            this.tabIndex = 0;
            const id = this.__id;
            if (id >= 0) {
                registerRelease(this, () => {
                    if (native.canvas && typeof native.canvas.destroy === "function") {
                        native.canvas.destroy(id);
                    }
                });
            }
        }

        __release() {
            if (this.__id < 0) return;
            unregisterRelease(this);
            const id = this.__id;
            this.__id = -1;
            if (native.canvas && typeof native.canvas.destroy === "function") {
                native.canvas.destroy(id);
            }
        }

        get style() {
            return this.__style;
        }

        set style(value) {
            if (typeof value === "string") this.__style.cssText = value;
        }

        get width() {
            return this.__width;
        }

        set width(value) {
            this.__resize(Math.max(1, Math.floor(Number(value) || 1)), this.__height);
        }

        get height() {
            return this.__height;
        }

        set height(value) {
            this.__resize(this.__width, Math.max(1, Math.floor(Number(value) || 1)));
        }

        __resize(width, height) {
            this.__width = width;
            this.__height = height;
            native.canvas.resize(this.__id, width, height);
            if (this.__gl) native.gl.resizeDrawingBuffer(width, height);
        }

        get offsetWidth() {
            return px(this.style.width) || screen.width;
        }

        get offsetHeight() {
            return px(this.style.height) || screen.height;
        }

        get parentElement() {
            return this.__parent;
        }

        getBoundingClientRect() {
            const rect = canvasSize(this);
            return {
                x: rect.x,
                y: rect.y,
                left: rect.x,
                top: rect.y,
                width: rect.width,
                height: rect.height,
                right: rect.x + rect.width,
                bottom: rect.y + rect.height,
            };
        }

        appendChild(child) {
            if (child instanceof HTMLCanvasElement) {
                if (child.__parent && child.__parent !== this && typeof child.__parent.removeChild === "function") {
                    child.__parent.removeChild(child);
                }
                child.__parent = this;
                registerTree(child);
                return child;
            }
            return child;
        }

        getContext(type) {
            if (type === "2d") {
                if (!this.__context2d) this.__context2d = new CanvasRenderingContext2D(this);
                return this.__context2d;
            }
            if (type === "webgl" || type === "experimental-webgl") {
                if (!this.__gl) {
                    this.__gl = new WebGLRenderingContext(this);
                    native.gl.resizeDrawingBuffer(this.__width, this.__height);
                }
                return this.__gl;
            }
            return null;
        }

        addEventListener(type, fn) {
            canvasListeners.add(type, fn);
        }

        removeEventListener(type, fn) {
            canvasListeners.remove(type, fn);
        }

        dispatchEvent(event) {
            canvasListeners.emit(event);
            return true;
        }

        focus() {
            document.activeElement = this;
        }

        blur() {
            if (document.activeElement === this) document.activeElement = null;
        }

        requestPointerLock() {
            native.window.setCursorLocked(true);
            document.pointerLockElement = this;
            documentListeners.emit(makeEvent("pointerlockchange", {}));
            return Promise.resolve();
        }

        requestFullscreen() {
            native.window.setFullscreen(true);
            document.fullscreenElement = this;
            documentListeners.emit(makeEvent("fullscreenchange", {}));
            return Promise.resolve();
        }

        webkitRequestFullscreen() {
            return this.requestFullscreen();
        }

        remove() {
            const parent = this.__parent;
            const quits = this === mainCanvas || Boolean(this.__gl && parent);
            if (parent && typeof parent.removeChild === "function") parent.removeChild(this);
            else this.__parent = null;
            if (quits) {
                mainCanvas = null;
                native.env.quit();
            }
        }

        __pngBytes() {
            let length = -1;
            let copy = null;
            if (this.__gl) {
                length = native.gl.screenshotLength();
                copy = (view) => native.gl.copyScreenshot(view);
            } else {
                length = native.canvas.pngLength(this.__id);
                copy = (view) => native.canvas.copyPng(this.__id, view);
            }
            if (length < 0) return null;
            const bytes = new Uint8Array(length);
            copy(bytes);
            return bytes;
        }

        toDataURL() {
            const bytes = this.__pngBytes();
            if (!bytes) return "data:,";
            let binary = "";
            for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
            return "data:image/png;base64," + btoa(binary);
        }

        toBlob(callback) {
            if (typeof callback !== "function") return;
            const bytes = this.__pngBytes();
            callback(new Blob([bytes || new Uint8Array(0)], { type: "image/png" }));
        }
    }

    // ---------------------------------------------------------------------- gl

    const GL = {
        DEPTH_BUFFER_BIT: 0x00000100,
        STENCIL_BUFFER_BIT: 0x00000400,
        COLOR_BUFFER_BIT: 0x00004000,
        POINTS: 0x0000,
        LINES: 0x0001,
        LINE_LOOP: 0x0002,
        LINE_STRIP: 0x0003,
        TRIANGLES: 0x0004,
        TRIANGLE_STRIP: 0x0005,
        TRIANGLE_FAN: 0x0006,
        ZERO: 0,
        ONE: 1,
        SRC_COLOR: 0x0300,
        ONE_MINUS_SRC_COLOR: 0x0301,
        SRC_ALPHA: 0x0302,
        ONE_MINUS_SRC_ALPHA: 0x0303,
        DST_ALPHA: 0x0304,
        ONE_MINUS_DST_ALPHA: 0x0305,
        DST_COLOR: 0x0306,
        ONE_MINUS_DST_COLOR: 0x0307,
        SRC_ALPHA_SATURATE: 0x0308,
        FUNC_ADD: 0x8006,
        BLEND_EQUATION: 0x8009,
        BLEND_EQUATION_RGB: 0x8009,
        BLEND_EQUATION_ALPHA: 0x883D,
        BLEND_DST_RGB: 0x80C8,
        BLEND_SRC_RGB: 0x80C9,
        BLEND_DST_ALPHA: 0x80CA,
        BLEND_SRC_ALPHA: 0x80CB,
        BLEND_COLOR: 0x8005,
        ARRAY_BUFFER: 0x8892,
        ELEMENT_ARRAY_BUFFER: 0x8893,
        ARRAY_BUFFER_BINDING: 0x8894,
        ELEMENT_ARRAY_BUFFER_BINDING: 0x8895,
        STREAM_DRAW: 0x88E0,
        STATIC_DRAW: 0x88E4,
        DYNAMIC_DRAW: 0x88E8,
        BUFFER_SIZE: 0x8764,
        BUFFER_USAGE: 0x8765,
        CURRENT_VERTEX_ATTRIB: 0x8626,
        FRONT: 0x0404,
        BACK: 0x0405,
        CULL_FACE: 0x0B44,
        BLEND: 0x0BE2,
        DITHER: 0x0BD0,
        STENCIL_TEST: 0x0B90,
        DEPTH_TEST: 0x0B71,
        SCISSOR_TEST: 0x0C11,
        POLYGON_OFFSET_FILL: 0x8037,
        SAMPLE_ALPHA_TO_COVERAGE: 0x809E,
        SAMPLE_COVERAGE: 0x80A0,
        NO_ERROR: 0,
        INVALID_ENUM: 0x0500,
        INVALID_VALUE: 0x0501,
        INVALID_OPERATION: 0x0502,
        OUT_OF_MEMORY: 0x0505,
        CONTEXT_LOST_WEBGL: 0x9242,
        CW: 0x0900,
        CCW: 0x0901,
        LINE_WIDTH: 0x0B21,
        ALIASED_LINE_WIDTH_RANGE: 0x846E,
        CULL_FACE_MODE: 0x0B45,
        FRONT_FACE: 0x0B46,
        DEPTH_RANGE: 0x0B70,
        DEPTH_WRITEMASK: 0x0B72,
        DEPTH_CLEAR_VALUE: 0x0B73,
        DEPTH_FUNC: 0x0B74,
        STENCIL_CLEAR_VALUE: 0x0B91,
        STENCIL_FUNC: 0x0B92,
        STENCIL_FAIL: 0x0B94,
        STENCIL_PASS_DEPTH_FAIL: 0x0B95,
        STENCIL_PASS_DEPTH_PASS: 0x0B96,
        STENCIL_REF: 0x0B97,
        STENCIL_VALUE_MASK: 0x0B93,
        STENCIL_WRITEMASK: 0x0B98,
        STENCIL_BACK_FUNC: 0x8800,
        STENCIL_BACK_FAIL: 0x8801,
        STENCIL_BACK_PASS_DEPTH_FAIL: 0x8802,
        STENCIL_BACK_PASS_DEPTH_PASS: 0x8803,
        STENCIL_BACK_REF: 0x8CA3,
        STENCIL_BACK_VALUE_MASK: 0x8CA4,
        STENCIL_BACK_WRITEMASK: 0x8CA5,
        STENCIL_BITS: 0x0D57,
        VIEWPORT: 0x0BA2,
        SCISSOR_BOX: 0x0C10,
        COLOR_CLEAR_VALUE: 0x0C22,
        COLOR_WRITEMASK: 0x0C23,
        UNPACK_ALIGNMENT: 0x0CF5,
        PACK_ALIGNMENT: 0x0D05,
        MAX_TEXTURE_SIZE: 0x0D33,
        MAX_VIEWPORT_DIMS: 0x0D3A,
        SUBPIXEL_BITS: 0x0D50,
        RED_BITS: 0x0D52,
        GREEN_BITS: 0x0D53,
        BLUE_BITS: 0x0D54,
        ALPHA_BITS: 0x0D55,
        DEPTH_BITS: 0x0D56,
        TEXTURE_2D: 0x0DE1,
        TEXTURE: 0x1702,
        TEXTURE0: 0x84C0,
        TEXTURE1: 0x84C1,
        TEXTURE2: 0x84C2,
        TEXTURE3: 0x84C3,
        TEXTURE4: 0x84C4,
        TEXTURE5: 0x84C5,
        TEXTURE6: 0x84C6,
        TEXTURE7: 0x84C7,
        TEXTURE8: 0x84C8,
        TEXTURE9: 0x84C9,
        TEXTURE10: 0x84CA,
        TEXTURE11: 0x84CB,
        TEXTURE12: 0x84CC,
        TEXTURE13: 0x84CD,
        TEXTURE14: 0x84CE,
        TEXTURE15: 0x84CF,
        TEXTURE16: 0x84D0,
        TEXTURE17: 0x84D1,
        TEXTURE18: 0x84D2,
        TEXTURE19: 0x84D3,
        TEXTURE20: 0x84D4,
        TEXTURE21: 0x84D5,
        TEXTURE22: 0x84D6,
        TEXTURE23: 0x84D7,
        TEXTURE24: 0x84D8,
        TEXTURE25: 0x84D9,
        TEXTURE26: 0x84DA,
        TEXTURE27: 0x84DB,
        TEXTURE28: 0x84DC,
        TEXTURE29: 0x84DD,
        TEXTURE30: 0x84DE,
        TEXTURE31: 0x84DF,
        TEXTURE_CUBE_MAP: 0x8513,
        TEXTURE_BINDING_CUBE_MAP: 0x8514,
        TEXTURE_MAG_FILTER: 0x2800,        TEXTURE_MIN_FILTER: 0x2801,
        TEXTURE_WRAP_S: 0x2802,
        TEXTURE_WRAP_T: 0x2803,
        REPEAT: 0x2901,
        CLAMP_TO_EDGE: 0x812F,
        MIRRORED_REPEAT: 0x8370,
        NEAREST: 0x2600,
        LINEAR: 0x2601,
        NEAREST_MIPMAP_NEAREST: 0x2700,
        LINEAR_MIPMAP_NEAREST: 0x2701,
        NEAREST_MIPMAP_LINEAR: 0x2702,
        LINEAR_MIPMAP_LINEAR: 0x2703,
        BYTE: 0x1400,
        UNSIGNED_BYTE: 0x1401,
        SHORT: 0x1402,
        UNSIGNED_SHORT: 0x1403,
        INT: 0x1404,
        UNSIGNED_INT: 0x1405,
        FLOAT: 0x1406,
        UNSIGNED_SHORT_4_4_4_4: 0x8033,
        UNSIGNED_SHORT_5_5_5_1: 0x8034,
        UNSIGNED_SHORT_5_6_5: 0x8363,
        ALPHA: 0x1906,
        RGB: 0x1907,
        RGBA: 0x1908,
        LUMINANCE: 0x1909,
        LUMINANCE_ALPHA: 0x190A,
        FRAGMENT_SHADER: 0x8B30,
        VERTEX_SHADER: 0x8B31,
        MAX_VERTEX_ATTRIBS: 0x8869,
        MAX_VERTEX_UNIFORM_VECTORS: 0x8DFB,
        MAX_VARYING_VECTORS: 0x8DFC,
        MAX_COMBINED_TEXTURE_IMAGE_UNITS: 0x8B4D,
        MAX_VERTEX_TEXTURE_IMAGE_UNITS: 0x8B4C,
        MAX_TEXTURE_IMAGE_UNITS: 0x8872,
        MAX_FRAGMENT_UNIFORM_VECTORS: 0x8DFD,
        SHADER_TYPE: 0x8B4F,
        DELETE_STATUS: 0x8B80,
        COMPILE_STATUS: 0x8B81,
        LINK_STATUS: 0x8B82,
        VALIDATE_STATUS: 0x8B83,
        ATTACHED_SHADERS: 0x8B85,
        ACTIVE_UNIFORMS: 0x8B86,
        ACTIVE_ATTRIBUTES: 0x8B89,
        SHADING_LANGUAGE_VERSION: 0x8B8C,
        CURRENT_PROGRAM: 0x8B8D,
        FRAMEBUFFER: 0x8D40,
        RENDERBUFFER: 0x8D41,
        RGBA4: 0x8056,
        RGB5_A1: 0x8057,
        RGB565: 0x8D62,
        DEPTH_COMPONENT16: 0x81A5,
        STENCIL_INDEX8: 0x8D48,
        DEPTH_STENCIL: 0x84F9,
        DEPTH_STENCIL_ATTACHMENT: 0x821A,
        COLOR_ATTACHMENT0: 0x8CE0,
        DEPTH_ATTACHMENT: 0x8D00,
        STENCIL_ATTACHMENT: 0x8D20,
        FRAMEBUFFER_COMPLETE: 0x8CD5,
        FRAMEBUFFER_INCOMPLETE_ATTACHMENT: 0x8CD6,
        FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT: 0x8CD7,
        FRAMEBUFFER_INCOMPLETE_DIMENSIONS: 0x8CD9,
        FRAMEBUFFER_UNSUPPORTED: 0x8CDD,
        FRAMEBUFFER_BINDING: 0x8CA6,
        RENDERBUFFER_BINDING: 0x8CA7,
        MAX_RENDERBUFFER_SIZE: 0x84E8,
        INVALID_FRAMEBUFFER_OPERATION: 0x0506,
        VERSION: 0x1F02,
        VENDOR: 0x1F00,
        RENDERER: 0x1F01,
        UNPACK_FLIP_Y_WEBGL: 0x9240,
        UNPACK_PREMULTIPLY_ALPHA_WEBGL: 0x9241,
        UNPACK_COLORSPACE_CONVERSION_WEBGL: 0x9243,
        BROWSER_DEFAULT_WEBGL: 0x9244,
        NEVER: 0x0200,
        LESS: 0x0201,
        EQUAL: 0x0202,
        LEQUAL: 0x0203,
        GREATER: 0x0204,
        NOTEQUAL: 0x0205,
        GEQUAL: 0x0206,
        ALWAYS: 0x0207,
        KEEP: 0x1E00,
        REPLACE: 0x1E01,
        INCR: 0x1E02,
        DECR: 0x1E03,
        INVERT: 0x150A,
        INCR_WRAP: 0x8507,
        DECR_WRAP: 0x8508,
    };

    class WebGLRenderingContext {
        constructor(canvas) {
            this.canvas = canvas;
            this.__uniforms = new Map();
            this.__attribs = new Map();
        }

        get drawingBufferWidth() {
            return native.gl.drawingBufferWidth();
        }

        get drawingBufferHeight() {
            return native.gl.drawingBufferHeight();
        }

        viewport(x, y, width, height) {
            native.gl.viewport(x, y, width, height);
        }

        clearColor(red, green, blue, alpha) {
            native.gl.clearColor(red, green, blue, alpha);
        }

        clear(mask) {
            native.gl.clear(mask);
        }

        enable(capability) {
            native.gl.enable(capability);
        }

        disable(capability) {
            native.gl.disable(capability);
        }

        blendFuncSeparate(srgb, drgb, salpha, dalpha) {
            native.gl.blendFuncSeparate(srgb, drgb, salpha, dalpha);
        }

        stencilFunc(func, reference, mask) {
            native.gl.stencilFunc(func, reference, mask);
        }

        stencilOp(fail, zfail, zpass) {
            native.gl.stencilOp(fail, zfail, zpass);
        }

        pixelStorei(parameter, value) {
            native.gl.pixelStorei(parameter, value ? 1 : 0);
        }

        getParameter(parameter) {
            return native.gl.getParameter(parameter);
        }

        activeTexture(unit) {
            native.gl.activeTexture(unit);
        }

        createTexture() {
            const id = native.gl.createTexture();
            return id < 0 ? null : id;
        }

        deleteTexture(texture) {
            if (texture != null) native.gl.deleteTexture(texture);
        }

        bindTexture(target, texture) {
            native.gl.bindTexture(target, texture == null ? -1 : texture);
        }

        texParameteri(target, parameter, value) {
            native.gl.texParameteri(target, parameter, value);
        }

        texImage2D(target, level, internalformat, ...rest) {
            if (rest.length >= 6) {
                const width = rest[0];
                const height = rest[1];
                const format = rest[3];
                const type = rest[4];
                const pixels = rest[5];
                native.gl.texImage2D(
                    target,
                    internalformat,
                    width,
                    height,
                    format,
                    type,
                    pixels == null ? null : bytesView(pixels),
                );
                return;
            }
            const format = rest[0];
            const type = rest[1];
            const source = rest[2];
            if (source instanceof HTMLCanvasElement) {
                native.gl.texImage2DCanvas(
                    target,
                    internalformat,
                    format,
                    type,
                    source.__id,
                );
            } else if (source instanceof HTMLVideoElement) {
                native.gl.texImage2DVideo(
                    target,
                    internalformat,
                    format,
                    type,
                    source.__nativeId,
                    source.currentTime,
                );
            } else if (source instanceof Image) {
                native.gl.texImage2DImage(target, internalformat, format, type, source.__id);
            } else if (source instanceof ImageData) {
                native.gl.texImage2D(
                    target,
                    internalformat,
                    source.width,
                    source.height,
                    format,
                    type,
                    bytesView(source.data),
                );
            } else if (source == null) {
                native.gl.texImage2D(target, internalformat, 0, 0, format, type, null);
            }
        }

        texSubImage2D(target, level, xoffset, yoffset, format, type, source) {
            if (source instanceof HTMLCanvasElement) {
                native.gl.texSubImage2DCanvas(
                    target,
                    xoffset,
                    yoffset,
                    format,
                    type,
                    source.__id,
                );
            } else if (source instanceof Image) {
                native.gl.texSubImage2DImage(
                    target,
                    xoffset,
                    yoffset,
                    format,
                    type,
                    source.__id,
                );
            } else if (source instanceof ImageData) {
                native.gl.texSubImage2D(
                    target,
                    xoffset,
                    yoffset,
                    source.width,
                    format,
                    type,
                    bytesView(source.data),
                );
            }
        }

        createBuffer() {
            const id = native.gl.createBuffer();
            return id < 0 ? null : id;
        }

        deleteBuffer(buffer) {
            if (buffer != null) native.gl.deleteBuffer(buffer);
        }

        bindBuffer(target, buffer) {
            native.gl.bindBuffer(target, buffer == null ? -1 : buffer);
        }

        bufferData(target, data, usage) {
            if (typeof data === "number") {
                native.gl.bufferDataSize(target, data, usage);
            } else {
                native.gl.bufferData(target, bytesView(data), usage);
            }
        }

        bufferSubData(target, offset, data) {
            native.gl.bufferSubData(target, offset, bytesView(data));
        }

        enableVertexAttribArray(index) {
            native.gl.enableVertexAttribArray(index);
        }

        disableVertexAttribArray(index) {
            native.gl.disableVertexAttribArray(index);
        }

        vertexAttribPointer(index, size, type, normalized, stride, offset) {
            native.gl.vertexAttribPointer(index, size, type, Boolean(normalized), stride, offset);
        }

        drawArrays(mode, first, count) {
            native.gl.drawArrays(mode, first, count);
        }

        drawElements(mode, count, type, offset) {
            native.gl.drawElements(mode, count, type, offset);
        }

        createShader(type) {
            const id = native.gl.createShader(type);
            return id < 0 ? null : id;
        }

        deleteShader(shader) {
            if (shader != null) native.gl.deleteShader(shader);
        }

        shaderSource(shader, source) {
            native.gl.shaderSource(shader, String(source));
        }

        compileShader(shader) {
            native.gl.compileShader(shader);
        }

        getShaderInfoLog(shader) {
            return shader == null ? "" : native.gl.getShaderInfoLog(shader);
        }

        getShaderParameter(shader, parameter) {
            if (shader == null) return false;
            if (parameter === GL.COMPILE_STATUS) {
                return !native.gl.getShaderInfoLog(shader);
            }
            return false;
        }

        createProgram() {
            const id = native.gl.createProgram();
            return id < 0 ? null : id;
        }

        deleteProgram(program) {
            if (program == null) return;
            this.__invalidateProgram(program);
            native.gl.deleteProgram(program);
        }

        attachShader(program, shader) {
            if (program != null && shader != null) native.gl.attachShader(program, shader);
        }

        bindAttribLocation(program, index, name) {
            native.gl.bindAttribLocation(program, index, String(name));
        }

        linkProgram(program) {
            this.__invalidateProgram(program);
            native.gl.linkProgram(program);
        }

        __invalidateProgram(program) {
            this.__uniforms.delete(Number(program));
            this.__attribs.delete(Number(program));
        }

        getProgramParameter(program, parameter) {
            if (program == null) return false;
            if (parameter === GL.LINK_STATUS) return native.gl.getLinkStatus(program);
            return 0;
        }

        getProgramInfoLog(program) {
            return program == null ? "" : native.gl.getProgramInfoLog(program);
        }

        useProgram(program) {
            native.gl.useProgram(program == null ? -1 : program);
        }

        getAttribLocation(program, name) {
            if (program == null) return -1;
            const key = Number(program);
            const nameKey = String(name);
            let cache = this.__attribs.get(key);
            if (cache && cache.has(nameKey)) return cache.get(nameKey);
            const location = native.gl.getAttribLocation(program, nameKey);
            if (!cache) {
                cache = new Map();
                this.__attribs.set(key, cache);
            }
            cache.set(nameKey, location);
            return location;
        }

        getUniformLocation(program, name) {
            if (program == null) return null;
            const key = Number(program);
            const nameKey = String(name);
            let cache = this.__uniforms.get(key);
            if (cache && cache.has(nameKey)) {
                const location = cache.get(nameKey);
                return location < 0 ? null : location;
            }
            const location = native.gl.getUniformLocation(program, nameKey);
            if (!cache) {
                cache = new Map();
                this.__uniforms.set(key, cache);
            }
            cache.set(nameKey, location);
            return location < 0 ? null : location;
        }

        __location(location) {
            return location == null ? -1 : location;
        }

        uniform1f(location, value) {
            native.gl.uniform1f(this.__location(location), value);
        }

        uniform1i(location, value) {
            native.gl.uniform1i(this.__location(location), value);
        }

        uniform2f(location, x, y) {
            native.gl.uniform2f(this.__location(location), x, y);
        }

        uniform3f(location, x, y, z) {
            native.gl.uniform3f(this.__location(location), x, y, z);
        }

        uniform1fv(location, value) {
            native.gl.uniform1fv(this.__location(location), float32(value));
        }

        uniform2fv(location, value) {
            native.gl.uniform2fv(this.__location(location), float32(value));
        }

        uniform3fv(location, value) {
            native.gl.uniform3fv(this.__location(location), float32(value));
        }

        uniformMatrix4fv(location, transpose, value) {
            native.gl.uniformMatrix4fv(this.__location(location), float32(value));
        }

        createFramebuffer() {
            const id = native.gl.createFramebuffer();
            return id < 0 ? null : id;
        }

        deleteFramebuffer(framebuffer) {
            if (framebuffer != null) native.gl.deleteFramebuffer(framebuffer);
        }

        bindFramebuffer(target, framebuffer) {
            native.gl.bindFramebuffer(target, framebuffer == null ? -1 : framebuffer);
        }

        createRenderbuffer() {
            const id = native.gl.createRenderbuffer();
            return id < 0 ? null : id;
        }

        deleteRenderbuffer(renderbuffer) {
            if (renderbuffer != null) native.gl.deleteRenderbuffer(renderbuffer);
        }

        bindRenderbuffer(target, renderbuffer) {
            native.gl.bindRenderbuffer(target, renderbuffer == null ? -1 : renderbuffer);
        }

        renderbufferStorage(target, internalformat, width, height) {
            native.gl.renderbufferStorage(target, internalformat, width, height);
        }

        framebufferTexture2D(target, attachment, textarget, texture, level) {
            native.gl.framebufferTexture2D(
                target,
                attachment,
                textarget,
                texture == null ? -1 : texture,
                level,
            );
        }

        framebufferRenderbuffer(target, attachment, renderbuffertarget, renderbuffer) {
            native.gl.framebufferRenderbuffer(
                target,
                attachment,
                renderbuffertarget,
                renderbuffer == null ? -1 : renderbuffer,
            );
        }

        checkFramebufferStatus(target) {
            return native.gl.checkFramebufferStatus(target);
        }

        readPixels(x, y, width, height, format, type, pixels) {
            native.gl.readPixels(x, y, width, height, bytesView(pixels));
        }

        getExtension(name) {
            if (name === "WEBGL_lose_context") {
                return { loseContext() {}, restoreContext() {} };
            }
            return null;
        }

        getSupportedExtensions() {
            return ["WEBGL_lose_context"];
        }

        getError() {
            return GL.NO_ERROR;
        }

        isContextLost() {
            return false;
        }

        flush() {}

        finish() {}
    }

    for (const [name, value] of Object.entries(GL)) {
        WebGLRenderingContext.prototype[name] = value;
    }

    // ------------------------------------------------------------------ images

    class Image {
        constructor(width, height) {
            this.__id = -1;
            this.__load = 0;
            this._src = "";
            this.width = Number(width) || 0;
            this.height = Number(height) || 0;
            this.naturalWidth = this.width;
            this.naturalHeight = this.height;
            this.onload = null;
            this.onerror = null;
            this.complete = false;
            this.crossOrigin = null;
        }

        __adopt(id) {
            this.__release();
            this.__id = id;
            if (id >= 0) {
                registerRelease(this, () => {
                    if (native.image && typeof native.image.free === "function") {
                        native.image.free(id);
                    }
                });
            }
        }

        __release() {
            if (this.__id < 0) return;
            unregisterRelease(this);
            const id = this.__id;
            this.__id = -1;
            if (native.image && typeof native.image.free === "function") native.image.free(id);
        }

        get src() {
            return this._src;
        }

        set src(value) {
            const url = String(value);
            this.__release();
            this._src = url;
            this.complete = false;
            const request = ++this.__load;
            const accept = (id) => {
                if (request !== this.__load) {
                    if (id >= 0 && native.image && typeof native.image.free === "function") {
                        native.image.free(id);
                    }
                    return;
                }
                if (id < 0) {
                    fail();
                    return;
                }
                this.__adopt(id);
                this.width = native.image.width(id);
                this.height = native.image.height(id);
                this.naturalWidth = this.width;
                this.naturalHeight = this.height;
                this.complete = true;
                setTimeoutFn(() => {
                    if (request !== this.__load) return;
                    if (typeof this.onload === "function") this.onload({ target: this });
                }, 0);
            };
            const fail = () => {
                if (request !== this.__load) return;
                this.__id = -1;
                this.width = 0;
                this.height = 0;
                this.naturalWidth = 0;
                this.naturalHeight = 0;
                this.complete = true;
                setTimeoutFn(() => {
                    if (request !== this.__load) return;
                    if (typeof this.onerror === "function") {
                        this.onerror(new Error("Failed to load image " + url));
                    }
                }, 0);
            };
            const apply = (bytes) => {
                let id = -1;
                try {
                    id = native.image.decode(bytes);
                } catch (error) {
                    reportError(error);
                }
                accept(id);
            };
            const data = decodeDataURL(url);
            if (data) {
                apply(data);
                return;
            }
            if (/^data:/i.test(url)) {
                fail();
                return;
            }
            if (/^blob:/i.test(url)) {
                const blob = objectURLs.get(url);
                if (blob) apply(blobBytes(blob));
                else fail();
                return;
            }
            try {
                const id = native.image.load(url);
                if (id >= 0) {
                    accept(id);
                    return;
                }
            } catch (error) {
                reportError(error);
            }
            if (!native.env.networkAllowed) {
                fail();
                return;
            }
            fetchResource(url)
                .then((response) => response.arrayBuffer())
                .then((buffer) => apply(new Uint8Array(buffer)))
                .catch(fail);
        }
    }

    class ImageData {
        constructor(a, b, c) {
            if (typeof a === "number") {
                this.width = a;
                this.height = b;
                this.data = new Uint8ClampedArray(a * b * 4);
            } else {
                this.data = a;
                this.width = b;
                this.height = c === undefined ? a.length / 4 / b : c;
                if (this.height === undefined) this.height = 0;
            }
        }
    }

    class Blob {
        constructor(parts, options) {
            this.__bytes = [];
            for (const part of parts || []) {
                if (part instanceof Uint8Array) this.__bytes.push(part);
                else if (part instanceof Blob) this.__bytes.push(part.__bytes[0] || new Uint8Array(0));
                else this.__bytes.push(new TextEncoder().encode(String(part)));
            }
            this.type = (options && options.type) || "";
            this.size = this.__bytes.reduce((sum, part) => sum + part.length, 0);
        }

        arrayBuffer() {
            const output = new Uint8Array(this.size);
            let offset = 0;
            for (const part of this.__bytes) {
                output.set(part, offset);
                offset += part.length;
            }
            return Promise.resolve(output.buffer);
        }

        text() {
            return this.arrayBuffer().then((buffer) => new TextDecoder().decode(buffer));
        }
    }

    const objectURLs = new Map();
    let nextObjectURL = 1;

    function randomBytes(length) {
        const bytes = new Uint8Array(length);
        for (let i = 0; i < length; i++) bytes[i] = Math.floor(Math.random() * 256);
        return bytes;
    }

    const crypto = {
        getRandomValues(view) {
            const bytes = new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
            for (let i = 0; i < bytes.length; i++) bytes[i] = Math.floor(Math.random() * 256);
            return view;
        },
        randomUUID() {
            const bytes = randomBytes(16);
            bytes[6] = (bytes[6] & 0x0f) | 0x40;
            bytes[8] = (bytes[8] & 0x3f) | 0x80;
            let hex = "";
            for (const byte of bytes) hex += byte.toString(16).padStart(2, "0");
            return (
                hex.slice(0, 8) +
                "-" +
                hex.slice(8, 12) +
                "-" +
                hex.slice(12, 16) +
                "-" +
                hex.slice(16, 20) +
                "-" +
                hex.slice(20)
            );
        },
    };

    function formDecode(value) {
        try {
            return decodeURIComponent(value.replace(/\+/g, "%20"));
        } catch {
            return value;
        }
    }

    function formEncode(value) {
        return encodeURIComponent(String(value))
            .replace(/%20/g, "+")
            .replace(/[!'()~]/g, (character) => "%" + character.charCodeAt(0).toString(16).toUpperCase());
    }

    class URLSearchParams {
        constructor(init) {
            this.__entries = [];
            if (init === undefined || init === null) return;
            if (typeof init === "string") {
                const text = init.replace(/^\?/, "");
                if (text === "") return;
                for (const pair of text.split("&")) {
                    if (pair === "") continue;
                    const separator = pair.indexOf("=");
                    const key = separator === -1 ? pair : pair.slice(0, separator);
                    const value = separator === -1 ? "" : pair.slice(separator + 1);
                    this.__entries.push([formDecode(key), formDecode(value)]);
                }
                return;
            }
            if (init instanceof URLSearchParams) {
                this.__entries = init.__entries.map((entry) => entry.slice());
                return;
            }
            if (typeof init[Symbol.iterator] === "function") {
                for (const pair of init) {
                    if (pair === null || pair === undefined) continue;
                    this.__entries.push([
                        String(pair[0]),
                        pair[1] === null || pair[1] === undefined ? "" : String(pair[1]),
                    ]);
                }
                return;
            }
            for (const key of Object.keys(init)) {
                this.__entries.push([key, String(init[key])]);
            }
        }

        get size() {
            return this.__entries.length;
        }

        append(key, value) {
            this.__entries.push([String(key), String(value)]);
        }

        delete(key) {
            const name = String(key);
            this.__entries = this.__entries.filter((entry) => entry[0] !== name);
        }

        get(key) {
            const name = String(key);
            const found = this.__entries.find((entry) => entry[0] === name);
            return found ? found[1] : null;
        }

        getAll(key) {
            const name = String(key);
            return this.__entries.filter((entry) => entry[0] === name).map((entry) => entry[1]);
        }

        has(key, value) {
            const name = String(key);
            if (arguments.length < 2) {
                return this.__entries.some((entry) => entry[0] === name);
            }
            const text = String(value);
            return this.__entries.some((entry) => entry[0] === name && entry[1] === text);
        }

        set(key, value) {
            const name = String(key);
            const text = String(value);
            const index = this.__entries.findIndex((entry) => entry[0] === name);
            this.__entries = this.__entries.filter((entry) => entry[0] !== name);
            if (index === -1) this.__entries.push([name, text]);
            else this.__entries.splice(index, 0, [name, text]);
        }

        sort() {
            this.__entries.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
        }

        toString() {
            return this.__entries
                .map((entry) => formEncode(entry[0]) + "=" + formEncode(entry[1]))
                .join("&");
        }

        forEach(callback, thisArg) {
            for (const entry of this.__entries.map((pair) => pair.slice())) {
                callback.call(thisArg, entry[1], entry[0], this);
            }
        }

        entries() {
            return this.__entries.map((entry) => entry.slice())[Symbol.iterator]();
        }

        keys() {
            return this.__entries.map((entry) => entry[0])[Symbol.iterator]();
        }

        values() {
            return this.__entries.map((entry) => entry[1])[Symbol.iterator]();
        }

        [Symbol.iterator]() {
            return this.entries();
        }
    }

    function normalizePathname(pathname) {
        const parts = [];
        for (const part of pathname.split("/")) {
            if (part === "" || part === ".") continue;
            if (part === "..") {
                parts.pop();
                continue;
            }
            parts.push(part);
        }
        const trailing = pathname.endsWith("/") && parts.length > 0;
        return "/" + parts.join("/") + (trailing ? "/" : "");
    }

    function parseURL(input, base) {
        let text = String(input).trim();
        if (base !== undefined && !/^[a-zA-Z][a-zA-Z0-9+.\-]*:/.test(text)) {
            const baseUrl = parseURL(base, undefined);
            if (!baseUrl) return null;
            const origin = baseUrl.protocol + "//" + baseUrl.host;
            if (text.startsWith("//")) {
                text = baseUrl.protocol + text;
            } else if (text.startsWith("/")) {
                text = origin + text;
            } else if (text.startsWith("?")) {
                text = origin + baseUrl.pathname + text;
            } else if (text.startsWith("#")) {
                text = origin + baseUrl.pathname + baseUrl.search + text;
            } else {
                const directory = baseUrl.pathname.slice(0, baseUrl.pathname.lastIndexOf("/") + 1);
                text = origin + directory + text;
            }
        }
        const scheme = /^([a-zA-Z][a-zA-Z0-9+.\-]*):/.exec(text);
        if (!scheme) return null;
        const protocol = scheme[1].toLowerCase() + ":";
        let rest = text.slice(scheme[0].length);
        const hasAuthority = rest.startsWith("//");
        let authority = "";
        if (hasAuthority) {
            rest = rest.slice(2);
            const end = rest.search(/[/?#]/);
            authority = end === -1 ? rest : rest.slice(0, end);
            rest = end === -1 ? "" : rest.slice(end);
        }
        let hash = "";
        const hashStart = rest.indexOf("#");
        if (hashStart !== -1) {
            hash = rest.slice(hashStart);
            rest = rest.slice(0, hashStart);
        }
        let search = "";
        const searchStart = rest.indexOf("?");
        if (searchStart !== -1) {
            search = rest.slice(searchStart);
            rest = rest.slice(0, searchStart);
        }
        let username = "";
        let password = "";
        let hostname = "";
        let port = "";
        if (hasAuthority) {
            let host = authority;
            const at = host.lastIndexOf("@");
            if (at !== -1) {
                const credentials = host.slice(0, at).split(":");
                username = credentials[0] ?? "";
                password = credentials[1] ?? "";
                host = host.slice(at + 1);
            }
            const colon = host.lastIndexOf(":");
            if (colon !== -1 && /^[0-9]*$/.test(host.slice(colon + 1))) {
                hostname = host.slice(0, colon).toLowerCase();
                port = host.slice(colon + 1);
            } else {
                hostname = host.toLowerCase();
            }
        }
        const host = port === "" ? hostname : hostname + ":" + port;
        const pathname = hasAuthority ? normalizePathname(rest === "" ? "/" : rest) : rest;
        const credentials = username === "" ? "" : username + (password === "" ? "" : ":" + password) + "@";
        const href = protocol + (hasAuthority ? "//" + credentials + host : "") + pathname + search + hash;
        return {
            href,
            protocol,
            username,
            password,
            host,
            hostname,
            port,
            pathname,
            search,
            hash,
            origin: hasAuthority ? protocol + "//" + host : "null",
        };
    }

    class URL {
        constructor(input, base) {
            const parsed = parseURL(
                input,
                base === undefined ? undefined : base instanceof URL ? base.href : String(base),
            );
            if (!parsed) throw new TypeError("Invalid URL: " + input);
            this.href = parsed.href;
            this.protocol = parsed.protocol;
            this.username = parsed.username;
            this.password = parsed.password;
            this.host = parsed.host;
            this.hostname = parsed.hostname;
            this.port = parsed.port;
            this.pathname = parsed.pathname;
            this.search = parsed.search;
            this.hash = parsed.hash;
            this.origin = parsed.origin;
        }

        get searchParams() {
            return new URLSearchParams(this.search);
        }

        toString() {
            return this.href;
        }

        toJSON() {
            return this.href;
        }

        static createObjectURL(blob) {
            const url = "blob:kaplay-native/" + nextObjectURL++;
            objectURLs.set(url, blob);
            return url;
        }

        static revokeObjectURL(url) {
            objectURLs.delete(url);
        }
    }

    // ------------------------------------------------------------------- fonts

    class FontFace {
        constructor(family, source) {
            this.family = family;
            this.source = source;
            this.status = "unloaded";
            this.loaded = null;
            if (source instanceof ArrayBuffer) this.__bytes = new Uint8Array(source);
            else if (ArrayBuffer.isView(source)) {
                this.__bytes = new Uint8Array(source.buffer, source.byteOffset, source.byteLength);
            } else {
                this.__bytes = null;
            }
        }

        load() {
            if (this.loaded) return this.loaded;
            if (this.__bytes) return this.__finish(this.__bytes);
            const source = String(this.source);
            const match = /url\((?:"|')?(.*?)(?:"|')?\)/.exec(source);
            const path = match ? match[1] : source;
            const data = decodeDataURL(path);
            if (data) return this.__finish(data);
            if (/^blob:/i.test(path)) {
                const blob = objectURLs.get(path);
                return blob ? this.__finish(blobBytes(blob)) : this.__fail();
            }
            const size = native.fs.size(path);
            if (size >= 0) {
                const bytes = new Uint8Array(size);
                if (native.fs.read(path, bytes)) return this.__finish(bytes);
            }
            if (!native.env.networkAllowed) return this.__fail();
            this.status = "loading";
            this.loaded = fetchResource(path)
                .then((response) => response.arrayBuffer())
                .then((buffer) => {
                    this.__register(new Uint8Array(buffer));
                    return this;
                })
                .catch(() => {
                    this.status = "error";
                    throw new Error("Failed to load font " + this.family);
                });
            return this.loaded;
        }

        __finish(bytes) {
            this.__register(bytes);
            this.loaded = Promise.resolve(this);
            return this.loaded;
        }

        __register(bytes) {
            native.font.register(this.family, bytes);
            this.status = "loaded";
        }

        __fail() {
            this.status = "error";
            this.loaded = Promise.reject(new Error("Failed to load font " + this.family));
            return this.loaded;
        }
    }

    const fontFaces = new Set();

    function parseFontFamily(spec) {
        const match = /(?:\d+)?(?:\.\d+)?px\s+(.*)$/.exec(String(spec));
        if (!match) return String(spec);
        return match[1]
            .split(",")[0]
            .trim()
            .replace(/^["']|["']$/g, "");
    }

    // ------------------------------------------------------------------- audio

    const audioBridge = native.audio || null;
    const audioVoices = new Set();
    const pendingDecodes = new Map();
    const graphNodes = [];
    const graphGains = [];
    const graphPans = [];
    const graphPanSet = [];
    const graphVisited = new Set();
    let automatingParams = 0;
    let nextAudioRequest = 1;

    function audioNow() {
        if (inFrame) return frameTime / 1000;
        if (audioBridge && typeof audioBridge.currentTime === "function") {
            return audioBridge.currentTime();
        }
        return native.env.now() / 1000;
    }

    function audioAvailable() {
        return Boolean(audioBridge && typeof audioBridge.play === "function");
    }

    function nextBufferId() {
        return audioBridge && typeof audioBridge.uploadBuffer === "function" ? audioBridge : null;
    }

    function syncAudioGraph() {
        for (const voice of audioVoices) {
            try {
                voice.__syncAudio();
            } catch (error) {
                reportError(error);
            }
        }
    }

    function audioTick() {
        if (!audioVoices.size || automatingParams === 0) return;
        syncAudioGraph();
    }

    function graphParams(node) {
        let gain = 1;
        let pan = 0;
        let reached = false;
        graphNodes.length = 0;
        graphGains.length = 0;
        graphPans.length = 0;
        graphPanSet.length = 0;
        graphVisited.clear();
        for (const out of node.__outs) {
            graphNodes.push(out);
            graphGains.push(1);
            graphPans.push(0);
            graphPanSet.push(false);
        }
        while (graphNodes.length) {
            const current = graphNodes.pop();
            const entryGain = graphGains.pop();
            const entryPan = graphPans.pop();
            const entryPanSet = graphPanSet.pop();
            if (!current || graphVisited.has(current)) continue;
            graphVisited.add(current);
            if (current.__isDestination) {
                reached = true;
                gain *= entryGain;
                if (entryPanSet) pan = entryPan;
                continue;
            }
            let nextGain = entryGain;
            let nextPan = entryPan;
            let panSet = entryPanSet;
            if (current instanceof GainNode) {
                nextGain *= current.gain.__currentValue();
            }
            if (current instanceof StereoPannerNode) {
                nextPan = current.pan.__currentValue();
                panSet = true;
            }
            for (const out of current.__outs) {
                graphNodes.push(out);
                graphGains.push(nextGain);
                graphPans.push(nextPan);
                graphPanSet.push(panSet);
            }
        }
        return { gain: reached ? gain : 0, pan };
    }

    class AudioParam {
        constructor(value, owner) {
            this.__value = Number(value) || 0;
            this.__owner = owner || null;
            this.__events = [];
            this.__automating = false;
        }

        get value() {
            return this.__currentValue();
        }

        set value(value) {
            this.__events = [];
            this.__idle();
            this.__value = Number(value) || 0;
            this.__notify();
        }

        __wake() {
            if (this.__automating) return;
            this.__automating = true;
            automatingParams++;
        }

        __idle() {
            if (!this.__automating) return;
            this.__automating = false;
            automatingParams--;
        }

        __notify() {
            if (this.__owner && typeof this.__owner.__paramChanged === "function") {
                this.__owner.__paramChanged();
            }
        }

        __currentValue() {
            const now = audioNow();
            this.__prune(now);
            return this.__valueAt(now);
        }

        __prune(now) {
            const events = this.__events;
            if (!events.length) {
                this.__idle();
                return;
            }
            let lastPast = -1;
            for (let index = 0; index < events.length; index++) {
                if (events[index].time > now) break;
                lastPast = index;
            }
            if (lastPast > 0) {
                events.splice(0, lastPast);
                lastPast = 0;
            }
            if (lastPast === events.length - 1) this.__idle();
            else this.__wake();
        }

        __valueAt(now) {
            if (!this.__events.length) return this.__value;
            let value = this.__value;
            let previousTime = -Infinity;
            for (const event of this.__events) {
                if (event.time <= now) {
                    previousTime = event.time;
                    value = event.value;
                    continue;
                }
                const span = event.time - previousTime;
                const progress = span > 0 ? (now - previousTime) / span : 1;
                if (event.type === "linear") {
                    return value + (event.value - value) * progress;
                }
                if (event.type === "exponential") {
                    const start = Math.abs(value) < 1e-8 ? 1e-8 : value;
                    const end = Math.abs(event.value) < 1e-8 ? 1e-8 : event.value;
                    return start * Math.pow(end / start, progress);
                }
                return value;
            }
            return value;
        }

        setValueAtTime(value, when) {
            this.__push("set", value, when);
            return this;
        }

        linearRampToValueAtTime(value, when) {
            if (!this.__events.length) this.__events.push({ type: "set", value: this.__value, time: 0 });
            this.__push("linear", value, when);
            return this;
        }

        exponentialRampToValueAtTime(value, when) {
            if (!this.__events.length) this.__events.push({ type: "set", value: this.__value, time: 0 });
            this.__push("exponential", value, when);
            return this;
        }

        setTargetAtTime(value, startTime, _timeConstant) {
            this.__push("set", value, startTime);
            return this;
        }

        setValueCurveAtTime(values, startTime) {
            const list = Array.from(values || []);
            const value = list.length ? list[list.length - 1] : this.__value;
            this.__push("set", value, startTime);
            return this;
        }

        cancelScheduledValues(when) {
            const time = Number(when) || 0;
            this.__events = this.__events.filter((event) => event.time < time);
            if (!this.__events.length) this.__idle();
            return this;
        }

        __push(type, value, when) {
            const time = Math.max(0, Number(when) || 0);
            this.__events.push({ type, value: Number(value) || 0, time });
            this.__events.sort((a, b) => a.time - b.time);
            this.__wake();
            this.__notify();
        }
    }

    class AudioNode {
        constructor(context) {
            this.context = context;
            this.__outs = [];
            this.__ins = [];
            this.__edges = [];
        }

        connect(node, output = 0, input = 0) {
            if (!node) return node;
            const edge = {
                node,
                output: Math.max(0, Number(output) || 0),
                input: Math.max(0, Number(input) || 0),
            };
            if (!this.__edges.some((item) => item.node === node && item.output === edge.output && item.input === edge.input)) {
                this.__edges.push(edge);
            }
            if (!this.__outs.includes(node)) {
                this.__outs.push(node);
                if (Array.isArray(node.__ins)) node.__ins.push(this);
            }
            syncAudioGraph();
            return node;
        }

        disconnect() {
            for (const out of this.__outs) {
                if (!Array.isArray(out.__ins)) continue;
                const index = out.__ins.indexOf(this);
                if (index >= 0) out.__ins.splice(index, 1);
            }
            this.__outs = [];
            this.__edges = [];
            syncAudioGraph();
        }
    }

    class AudioDestinationNode extends AudioNode {
        constructor(context) {
            super(context);
            this.__isDestination = true;
            this.maxChannelCount = 2;
        }
    }

    class GainNode extends AudioNode {
        constructor(context) {
            super(context);
            this.gain = new AudioParam(1, this);
        }

        __paramChanged() {
            syncAudioGraph();
        }
    }

    class StereoPannerNode extends AudioNode {
        constructor(context) {
            super(context);
            this.pan = new AudioParam(0, this);
        }

        __paramChanged() {
            syncAudioGraph();
        }
    }

    class AudioBuffer {
        constructor(options) {
            const settings = options || {};
            this.length = Number(settings.length) || 1;
            this.sampleRate = Number(settings.sampleRate) || 44100;
            this.numberOfChannels = Number(settings.numberOfChannels) || 1;
            this.duration = this.length / this.sampleRate;
            this.__nativeId = Number(settings.__nativeId) || 0;
            this.__channels = null;
            this.__releaseRegistered = false;
            if (!this.__nativeId) {
                this.__channels = [];
                for (let i = 0; i < this.numberOfChannels; i++) {
                    this.__channels.push(new Float32Array(this.length));
                }
            } else {
                this.__registerRelease();
            }
        }

        __registerRelease() {
            if (this.__releaseRegistered || this.__nativeId <= 0) return;
            const id = this.__nativeId;
            registerRelease(this, () => {
                if (audioBridge && typeof audioBridge.freeBuffer === "function") {
                    audioBridge.freeBuffer(id);
                }
            });
            this.__releaseRegistered = true;
        }

        __disown() {
            if (!this.__releaseRegistered) return;
            unregisterRelease(this);
            this.__releaseRegistered = false;
        }

        getChannelData(channel) {
            if (!this.__channels) {
                const channels = [];
                for (let index = 0; index < this.numberOfChannels; index++) {
                    const data = new Float32Array(this.length);
                    if (this.__nativeId && audioBridge && audioBridge.readChannel) {
                        audioBridge.readChannel(this.__nativeId, index, data);
                    }
                    channels.push(data);
                }
                this.__channels = channels;
            }
            return this.__channels[channel] || this.__channels[0];
        }

        copyFromChannel(destination, channel, start) {
            const source = this.getChannelData(channel);
            const offset = start || 0;
            destination.set(source.subarray(offset, offset + destination.length));
        }

        copyToChannel(source, channel, start) {
            this.getChannelData(channel).set(source, start || 0);
        }

        __upload() {
            if (this.__nativeId) return this.__nativeId;
            if (!nextBufferId()) return 0;
            const channels = this.numberOfChannels;
            const interleaved = new Float32Array(this.length * channels);
            for (let frame = 0; frame < this.length; frame++) {
                for (let channel = 0; channel < channels; channel++) {
                    interleaved[frame * channels + channel] = this.__channels[channel][frame];
                }
            }
            this.__nativeId = audioBridge.uploadBuffer(this.sampleRate, channels, interleaved);
            this.__registerRelease();
            return this.__nativeId;
        }
    }

    class AudioBufferSourceNode extends AudioNode {
        constructor(context) {
            super(context);
            this.buffer = null;
            this.__loop = false;
            this.loopStart = 0;
            this.loopEnd = 0;
            this.playbackRate = new AudioParam(1, this);
            this.detune = new AudioParam(0, this);
            this.onended = null;
            this.__voice = -1;
            this.__started = false;
            this.__stopped = false;
            this.__offset = 0;
            this.__startTime = 0;
            this.__stopTime = -1;
            this.__listeners = makeListeners();
            this.__ownedBuffers = [];
            this.__renderedGraph = false;
        }

        get loop() {
            return this.__loop;
        }

        set loop(value) {
            this.__loop = Boolean(value);
            this.__paramChanged();
        }

        start(when = 0, offset = 0, duration) {
            if (this.__started) {
                throw new Error("cannot call start more than once");
            }
            this.__started = true;
            this.__stopped = false;
            this.__offset = Math.max(0, Number(offset) || 0);
            const time = Number(when) || 0;
            const delay = Math.max(0, time - audioNow());
            this.__startTime = audioNow() + delay;
            if (duration !== undefined) {
                this.__stopTime = this.__startTime + (Number(duration) || 0);
            }
            if (this.buffer) {
                if (needsGraphRender(this)) {
                    setTimeoutFn(() => this.__createVoice(), 0);
                } else {
                    this.__createVoice();
                }
            }
        }

        stop(when = 0) {
            if (!this.__started) return;
            const time = Number(when) || 0;
            this.__stopTime = time;
            if (this.__voice >= 0) {
                if (time > audioNow()) {
                    if (audioBridge && audioBridge.stopAt) {
                        audioBridge.stopAt(this.__voice, time * this.context.sampleRate);
                    }
                    this.__stopTime = time;
                } else {
                    audioBridge.stop(this.__voice);
                    this.__voice = -1;
                    audioVoices.delete(this);
                    this.__fireEnded();
                }
            }
            this.__stopped = true;
        }

        __createVoice() {
            if (!audioAvailable()) return;
            const fallback = this instanceof OscillatorNode ? 2 : 30;
            const duration = this.__stopTime > this.__startTime
                ? this.__stopTime - this.__startTime
                : (this.buffer ? this.buffer.duration : fallback);
            let buffer = this.buffer;
            let rendered = null;
            if (needsGraphRender(this)) {
                rendered = renderGraph(this, this.__startTime, duration);
                if (rendered) buffer = rendered;
            }
            if (!buffer) return;
            const nativeId = buffer.__upload();
            if (!nativeId) return;
            if (rendered) {
                buffer.__disown();
                this.__ownedBuffers.push(nativeId);
            }
            this.__renderedGraph = Boolean(rendered);
            const params = this.__renderedGraph ? { gain: 1, pan: 0 } : graphParams(this);
            const startFrame = this.__startTime > audioNow() ? this.__startTime * this.context.sampleRate : -1;
            let voice;
            try {
                voice = audioBridge.play(
                    nativeId,
                    params.gain,
                    params.pan,
                    this.playbackRate.__currentValue(),
                    this.loop,
                    rendered ? 0 : this.__offset,
                    startFrame,
                );
            } catch (error) {
                for (const id of this.__ownedBuffers.splice(0)) {
                    if (audioBridge && audioBridge.freeBuffer) audioBridge.freeBuffer(id);
                }
                throw error;
            }
            if (voice < 0) {
                for (const id of this.__ownedBuffers.splice(0)) {
                    if (audioBridge && audioBridge.freeBuffer) audioBridge.freeBuffer(id);
                }
                return;
            }
            this.__voice = voice;
            audioVoices.add(this);
            if (this.__stopTime >= 0) {
                audioBridge.stopAt(voice, this.__stopTime * this.context.sampleRate);
            }
        }

        __syncAudio() {
            if (this.__voice < 0) return;
            const params = this.__renderedGraph ? { gain: 1, pan: 0 } : graphParams(this);
            audioBridge.setGain(this.__voice, params.gain);
            audioBridge.setPan(this.__voice, params.pan);
            audioBridge.setRate(this.__voice, this.playbackRate.__currentValue());
            audioBridge.setLoop(this.__voice, this.loop);
        }

        __paramChanged() {
            if (this.__voice >= 0) this.__syncAudio();
        }

        __fireEnded() {
            for (const id of this.__ownedBuffers.splice(0)) {
                if (audioBridge && audioBridge.freeBuffer) audioBridge.freeBuffer(id);
            }
            const event = makeEvent("ended", {});
            event.target = this;
            try {
                if (typeof this.onended === "function") this.onended(event);
            } catch (error) {
                reportError(error);
            }
            this.__listeners.emit(event);
        }

        addEventListener(type, fn) {
            this.__listeners.add(type, fn);
        }

        removeEventListener(type, fn) {
            this.__listeners.remove(type, fn);
        }
    }

    class BiquadFilterNode extends AudioNode {
        constructor(context) {
            super(context);
            this.type = "lowpass";
            this.frequency = new AudioParam(350, this);
            this.detune = new AudioParam(0, this);
            this.Q = new AudioParam(1, this);
            this.gain = new AudioParam(0, this);
        }
    }

    class DelayNode extends AudioNode {
        constructor(context, maxDelayTime = 1) {
            super(context);
            this.__maxDelayTime = Math.max(0, Math.min(180, Number(maxDelayTime) || 0));
            this.delayTime = new AudioParam(0, this);
        }
    }

    class WaveShaperNode extends AudioNode {
        constructor(context) {
            super(context);
            this.__curve = null;
            this.oversample = "none";
        }

        get curve() {
            return this.__curve;
        }

        set curve(value) {
            if (value === null || value === undefined) {
                this.__curve = null;
                return;
            }
            this.__curve = Float32Array.from(value);
        }
    }

    class DynamicsCompressorNode extends AudioNode {
        constructor(context) {
            super(context);
            this.threshold = new AudioParam(-24, this);
            this.knee = new AudioParam(30, this);
            this.ratio = new AudioParam(12, this);
            this.attack = new AudioParam(0.003, this);
            this.release = new AudioParam(0.25, this);
            this.reduction = 0;
        }
    }

    class ConvolverNode extends AudioNode {
        constructor(context) {
            super(context);
            this.__buffer = null;
            this.normalize = true;
        }

        get buffer() {
            return this.__buffer;
        }

        set buffer(value) {
            this.__buffer = value || null;
        }
    }

    class ChannelSplitterNode extends AudioNode {
        constructor(context, numberOfOutputs = 6) {
            super(context);
            this.numberOfOutputs = Math.max(1, Math.min(32, Math.round(Number(numberOfOutputs) || 6)));
        }
    }

    class ChannelMergerNode extends AudioNode {
        constructor(context, numberOfInputs = 6) {
            super(context);
            this.numberOfInputs = Math.max(1, Math.min(32, Math.round(Number(numberOfInputs) || 6)));
        }
    }

    class IIRFilterNode extends AudioNode {
        constructor(context, feedforward, feedback) {
            super(context);
            const feed = Float32Array.from(feedforward || []);
            const back = Float32Array.from(feedback || []);
            if (!feed.length || feed.length > 20 || !back.length || back.length > 20) {
                throw new Error("IIR filters require 1 to 20 coefficients");
            }
            if (Math.abs(back[0]) < 1e-12) throw new Error("IIR feedback[0] must not be zero");
            this.__feedforward = feed;
            this.__feedback = back;
        }

        getFrequencyResponse(frequencyHz, magResponse, phaseResponse) {
            const length = Math.min(frequencyHz.length, magResponse.length, phaseResponse.length);
            for (let index = 0; index < length; index++) {
                const omega = 2 * Math.PI * frequencyHz[index] / this.context.sampleRate;
                let numeratorReal = 0;
                let numeratorImag = 0;
                let denominatorReal = 0;
                let denominatorImag = 0;
                for (let k = 0; k < this.__feedforward.length; k++) {
                    numeratorReal += this.__feedforward[k] * Math.cos(-omega * k);
                    numeratorImag += this.__feedforward[k] * Math.sin(-omega * k);
                }
                for (let k = 0; k < this.__feedback.length; k++) {
                    denominatorReal += this.__feedback[k] * Math.cos(-omega * k);
                    denominatorImag += this.__feedback[k] * Math.sin(-omega * k);
                }
                const denominator = denominatorReal * denominatorReal + denominatorImag * denominatorImag;
                const real = (numeratorReal * denominatorReal + numeratorImag * denominatorImag) / (denominator || 1e-12);
                const imag = (numeratorImag * denominatorReal - numeratorReal * denominatorImag) / (denominator || 1e-12);
                magResponse[index] = Math.hypot(real, imag);
                phaseResponse[index] = Math.atan2(imag, real);
            }
        }
    }

    class PannerNode extends AudioNode {
        constructor(context) {
            super(context);
            this.positionX = new AudioParam(0, this);
            this.positionY = new AudioParam(0, this);
            this.positionZ = new AudioParam(0, this);
            this.orientationX = new AudioParam(1, this);
            this.orientationY = new AudioParam(0, this);
            this.orientationZ = new AudioParam(0, this);
            this.panningModel = "equalpower";
            this.distanceModel = "inverse";
            this.refDistance = 1;
            this.maxDistance = 10000;
            this.rolloffFactor = 1;
            this.coneInnerAngle = 360;
            this.coneOuterAngle = 360;
            this.coneOuterGain = 0;
        }
    }

    class ConstantSourceNode extends AudioNode {
        constructor(context) {
            super(context);
            this.offset = new AudioParam(1, this);
            this.onended = null;
            this.__voice = -1;
            this.__started = false;
            this.__startTime = 0;
            this.__stopTime = -1;
            this.__ownedBuffers = [];
            this.__listeners = makeListeners();
        }

        start(when = 0) {
            if (this.__started) throw new Error("cannot call start more than once");
            this.__started = true;
            this.__startTime = Math.max(audioNow(), Number(when) || 0);
            setTimeoutFn(() => this.__createVoice(), 0);
        }

        stop(when = 0) {
            if (!this.__started) return;
            const time = Number(when) || 0;
            this.__stopTime = time;
            if (this.__voice >= 0) {
                if (time > audioNow() && audioBridge && audioBridge.stopAt) {
                    audioBridge.stopAt(this.__voice, time * this.context.sampleRate);
                } else {
                    audioBridge.stop(this.__voice);
                    this.__voice = -1;
                    audioVoices.delete(this);
                    this.__fireEnded();
                }
            }
        }

        __duration() {
            if (this.__stopTime > this.__startTime) return this.__stopTime - this.__startTime;
            return 30;
        }

        __createVoice() {
            if (!audioAvailable()) return;
            const buffer = renderGraph(this, this.__startTime, this.__duration());
            if (!buffer) return;
            const nativeId = buffer.__upload();
            if (!nativeId) return;
            buffer.__disown();
            this.__ownedBuffers.push(nativeId);
            let voice;
            try {
                voice = audioBridge.play(nativeId, 1, 0, 1, this.__stopTime < 0, 0, this.__startTime > audioNow() ? this.__startTime * this.context.sampleRate : -1);
            } catch (error) {
                for (const id of this.__ownedBuffers.splice(0)) {
                    if (audioBridge && audioBridge.freeBuffer) audioBridge.freeBuffer(id);
                }
                throw error;
            }
            if (voice < 0) {
                for (const id of this.__ownedBuffers.splice(0)) {
                    if (audioBridge && audioBridge.freeBuffer) audioBridge.freeBuffer(id);
                }
                return;
            }
            this.__voice = voice;
            audioVoices.add(this);
            if (this.__stopTime >= 0) audioBridge.stopAt(voice, this.__stopTime * this.context.sampleRate);
        }

        __syncAudio() {}

        addEventListener(type, fn) {
            this.__listeners.add(type, fn);
        }

        removeEventListener(type, fn) {
            this.__listeners.remove(type, fn);
        }

        __fireEnded() {
            for (const id of this.__ownedBuffers.splice(0)) {
                if (audioBridge && audioBridge.freeBuffer) audioBridge.freeBuffer(id);
            }
            const event = makeEvent("ended", {});
            event.target = this;
            try {
                if (typeof this.onended === "function") this.onended(event);
            } catch (error) {
                reportError(error);
            }
            this.__listeners.emit(event);
        }
    }

    class AnalyserNode extends AudioNode {
        constructor(context) {
            super(context);
            this.__fftSize = 2048;
            this.minDecibels = -100;
            this.maxDecibels = -30;
            this.smoothingTimeConstant = 0.8;
            this.__smoothed = null;
        }

        get fftSize() {
            return this.__fftSize;
        }

        set fftSize(value) {
            const size = Math.round(Number(value) || 0);
            if (size < 32 || size > 32768 || (size & (size - 1)) !== 0) {
                throw new Error("fftSize must be a power of two between 32 and 32768");
            }
            this.__fftSize = size;
            this.__smoothed = null;
        }

        get frequencyBinCount() {
            return this.__fftSize >> 1;
        }

        __timeData() {
            const data = new Float32Array(this.__fftSize);
            if (audioBridge && typeof audioBridge.analyser === "function") {
                try {
                    audioBridge.analyser(data);
                } catch (error) {
                    reportError(error);
                }
            }
            return data;
        }

        __frequencies() {
            const data = this.__timeData();
            const size = this.__fftSize;
            const real = Float64Array.from(data);
            const imag = new Float64Array(size);
            for (let index = 0; index < size; index++) {
                real[index] *= 0.5 - 0.5 * Math.cos(2 * Math.PI * index / size);
            }
            fftInPlace(real, imag);
            const bins = this.frequencyBinCount;
            const magnitudes = new Float32Array(bins);
            const smoothing = Math.max(0, Math.min(1, Number(this.smoothingTimeConstant) || 0));
            const previous = this.__smoothed;
            for (let index = 0; index < bins; index++) {
                const magnitude = Math.hypot(real[index], imag[index]);
                magnitudes[index] = previous && previous.length === bins
                    ? previous[index] * smoothing + magnitude * (1 - smoothing)
                    : magnitude;
            }
            this.__smoothed = magnitudes;
            const decibels = new Float32Array(bins);
            for (let index = 0; index < bins; index++) {
                decibels[index] = 20 * Math.log10(Math.max(1e-8, magnitudes[index]));
            }
            return decibels;
        }

        getFloatFrequencyData(array) {
            const decibels = this.__frequencies();
            array.set(decibels.subarray(0, Math.min(array.length, decibels.length)));
        }

        getByteFrequencyData(array) {
            const decibels = this.__frequencies();
            const min = Number(this.minDecibels);
            const max = Number(this.maxDecibels);
            const span = Math.max(1e-6, max - min);
            for (let index = 0; index < Math.min(array.length, decibels.length); index++) {
                array[index] = Math.max(0, Math.min(255, Math.round((decibels[index] - min) / span * 255)));
            }
        }

        getFloatTimeDomainData(array) {
            const data = this.__timeData();
            array.set(data.subarray(0, Math.min(array.length, data.length)));
        }

        getByteTimeDomainData(array) {
            const data = this.__timeData();
            for (let index = 0; index < Math.min(array.length, data.length); index++) {
                array[index] = Math.max(0, Math.min(255, Math.round((data[index] + 1) * 128)));
            }
        }
    }

    class PeriodicWave {
        constructor(real, imag, options) {
            const settings = options || {};
            this.__real = Float32Array.from(real || []);
            this.__imag = Float32Array.from(imag || []);
            if (this.__real.length < 2 || this.__real.length !== this.__imag.length) {
                throw new Error("PeriodicWave real and imaginary arrays must have the same length of at least 2");
            }
            this.__disableNormalization = Boolean(settings.disableNormalization);
        }
    }

    function fftInPlace(real, imag) {
        const length = real.length;
        for (let index = 1, reverse = 0; index < length; index++) {
            let bit = length >> 1;
            for (; reverse & bit; bit >>= 1) reverse ^= bit;
            reverse ^= bit;
            if (index < reverse) {
                [real[index], real[reverse]] = [real[reverse], real[index]];
                [imag[index], imag[reverse]] = [imag[reverse], imag[index]];
            }
        }
        for (let size = 2; size <= length; size <<= 1) {
            const half = size >> 1;
            const angle = -2 * Math.PI / size;
            for (let start = 0; start < length; start += size) {
                for (let offset = 0; offset < half; offset++) {
                    const cosine = Math.cos(angle * offset);
                    const sine = Math.sin(angle * offset);
                    const even = start + offset;
                    const odd = even + half;
                    const oddReal = real[odd] * cosine - imag[odd] * sine;
                    const oddImag = real[odd] * sine + imag[odd] * cosine;
                    real[odd] = real[even] - oddReal;
                    imag[odd] = imag[even] - oddImag;
                    real[even] += oddReal;
                    imag[even] += oddImag;
                }
            }
        }
    }

    const MAX_RENDER_SECONDS = 30;

    function graphNodeNeedsRender(node) {
        return node instanceof BiquadFilterNode
            || node instanceof DelayNode
            || node instanceof WaveShaperNode
            || node instanceof DynamicsCompressorNode
            || node instanceof ConvolverNode
            || node instanceof ChannelSplitterNode
            || node instanceof ChannelMergerNode
            || node instanceof IIRFilterNode
            || node instanceof PannerNode;
    }

    function needsGraphRender(source) {
        if (source instanceof MediaElementAudioSourceNode) return false;
        if (source instanceof OscillatorNode || source instanceof ConstantSourceNode) return true;
        let needed = false;
        const visit = (node, visited) => {
            if (!node || visited.has(node) || needed) return;
            visited.add(node);
            if (graphNodeNeedsRender(node)) {
                needed = true;
                return;
            }
            for (const edge of node.__edges) visit(edge.node, visited);
        };
        for (const edge of source.__edges) visit(edge.node, new Set());
        return needed;
    }

    function graphOrder(source) {
        const order = [];
        const visiting = new Set();
        const done = new Set();
        const visit = (node) => {
            if (done.has(node)) return;
            if (visiting.has(node)) throw new Error("audio feedback loops are not supported by the native graph renderer");
            visiting.add(node);
            for (const edge of node.__edges) visit(edge.node);
            visiting.delete(node);
            done.add(node);
            order.push(node);
        };
        visit(source);
        return order.reverse();
    }

    function silenceBus(channels, length) {
        return Array.from({ length: Math.max(1, channels) }, () => new Float32Array(length));
    }

    function sumInputBuses(buses, length, channels = 1) {
        if (!buses || !buses.length) return silenceBus(channels, length);
        let count = Math.max(1, channels);
        for (const bus of buses) count = Math.max(count, bus.length);
        const out = silenceBus(count, length);
        for (const bus of buses) {
            if (bus.length === 1 && count > 1) {
                for (let index = 0; index < length; index++) {
                    const sample = bus[0][index];
                    for (let channel = 0; channel < count; channel++) out[channel][index] += sample;
                }
                continue;
            }
            for (let channel = 0; channel < Math.min(bus.length, count); channel++) {
                const input = bus[channel];
                const target = out[channel];
                for (let index = 0; index < length; index++) target[index] += input[index];
            }
        }
        return out;
    }

    function oscillatorSample(node, phase) {
        const fraction = phase - Math.floor(phase);
        if (node.type === "square") return fraction < 0.5 ? 1 : -1;
        if (node.type === "sawtooth") return 2 * fraction - 1;
        if (node.type === "triangle") return 1 - 4 * Math.abs(fraction - 0.5);
        if (node.type === "custom") {
            const wave = node.__periodicWave;
            if (!wave) return 0;
            let value = 0;
            for (let harmonic = 1; harmonic < wave.__real.length; harmonic++) {
                const angle = 2 * Math.PI * harmonic * fraction;
                value += wave.__real[harmonic] * Math.cos(angle) + wave.__imag[harmonic] * Math.sin(angle);
            }
            return value;
        }
        return Math.sin(2 * Math.PI * fraction);
    }

    function renderSourceOutput(source, start, length, rate) {
        if (source instanceof ConstantSourceNode) {
            const output = new Float32Array(length);
            for (let index = 0; index < length; index++) output[index] = source.offset.__valueAt(start + index / rate);
            return [output];
        }
        if (source instanceof OscillatorNode) {
            const output = new Float32Array(length);
            let phase = 0;
            for (let index = 0; index < length; index++) {
                const time = start + index / rate;
                output[index] = oscillatorSample(source, phase);
                const frequency = Math.max(0, source.frequency.__valueAt(time));
                phase += frequency * Math.pow(2, source.detune.__valueAt(time) / 1200) / rate;
            }
            return [output];
        }
        const buffer = source.buffer;
        if (!buffer) return silenceBus(1, length);
        const channels = Math.min(2, Math.max(1, buffer.numberOfChannels));
        const inputs = [];
        for (let channel = 0; channel < channels; channel++) inputs.push(buffer.getChannelData(channel));
        const outputs = silenceBus(channels, length);
        const loop = Boolean(source.loop);
        const loopStart = Math.max(0, source.loopStart || 0) * rate;
        const loopEnd = source.loopEnd > 0 ? Math.min(buffer.length, source.loopEnd * rate) : buffer.length;
        let position = Math.max(0, source.__offset || 0) * rate;
        for (let index = 0; index < length; index++) {
            const time = start + index / rate;
            const speed = Math.max(0, source.playbackRate.__valueAt(time)) * Math.pow(2, source.detune.__valueAt(time) / 1200);
            if (loop && loopEnd > loopStart) {
                const span = loopEnd - loopStart;
                while (position >= loopEnd) position -= span;
                while (position < loopStart) position += span;
            }
            if (position >= 0 && position < buffer.length) {
                const frame = Math.floor(position);
                for (let channel = 0; channel < channels; channel++) outputs[channel][index] = inputs[channel][frame] || 0;
            }
            position += speed;
        }
        return outputs;
    }

    function biquadCoefficients(node, time, rate) {
        const frequency = Math.max(1, Math.min(rate * 0.499, node.frequency.__valueAt(time) * Math.pow(2, node.detune.__valueAt(time) / 1200)));
        const omega = 2 * Math.PI * frequency / rate;
        const cosine = Math.cos(omega);
        const sine = Math.sin(omega);
        const q = Math.max(0.0001, node.Q.__valueAt(time));
        const alpha = sine / (2 * q);
        const gain = node.gain.__valueAt(time);
        const a = Math.pow(10, gain / 40);
        let b0 = 1;
        let b1 = 0;
        let b2 = 0;
        let a0 = 1;
        let a1 = 0;
        let a2 = 0;
        if (node.type === "lowpass") {
            b0 = (1 - cosine) / 2;
            b1 = 1 - cosine;
            b2 = b0;
            a0 = 1 + alpha;
            a1 = -2 * cosine;
            a2 = 1 - alpha;
        } else if (node.type === "highpass") {
            b0 = (1 + cosine) / 2;
            b1 = -1 - cosine;
            b2 = b0;
            a0 = 1 + alpha;
            a1 = -2 * cosine;
            a2 = 1 - alpha;
        } else if (node.type === "bandpass") {
            b0 = alpha;
            b1 = 0;
            b2 = -alpha;
            a0 = 1 + alpha;
            a1 = -2 * cosine;
            a2 = 1 - alpha;
        } else if (node.type === "notch") {
            b0 = 1;
            b1 = -2 * cosine;
            b2 = 1;
            a0 = 1 + alpha;
            a1 = -2 * cosine;
            a2 = 1 - alpha;
        } else if (node.type === "allpass") {
            b0 = 1 - alpha;
            b1 = -2 * cosine;
            b2 = 1 + alpha;
            a0 = 1 + alpha;
            a1 = -2 * cosine;
            a2 = 1 - alpha;
        } else if (node.type === "peaking") {
            b0 = 1 + alpha * a;
            b1 = -2 * cosine;
            b2 = 1 - alpha * a;
            a0 = 1 + alpha / a;
            a1 = -2 * cosine;
            a2 = 1 - alpha / a;
        } else if (node.type === "lowshelf") {
            const beta = 2 * Math.sqrt(a) * alpha;
            b0 = a * ((a + 1) - (a - 1) * cosine + beta);
            b1 = 2 * a * ((a - 1) - (a + 1) * cosine);
            b2 = a * ((a + 1) - (a - 1) * cosine - beta);
            a0 = (a + 1) + (a - 1) * cosine + beta;
            a1 = -2 * ((a - 1) + (a + 1) * cosine);
            a2 = (a + 1) + (a - 1) * cosine - beta;
        } else if (node.type === "highshelf") {
            const beta = 2 * Math.sqrt(a) * alpha;
            b0 = a * ((a + 1) + (a - 1) * cosine + beta);
            b1 = -2 * a * ((a - 1) + (a + 1) * cosine);
            b2 = a * ((a + 1) + (a - 1) * cosine - beta);
            a0 = (a + 1) - (a - 1) * cosine + beta;
            a1 = 2 * ((a - 1) - (a + 1) * cosine);
            a2 = (a + 1) - (a - 1) * cosine - beta;
        } else {
            throw new Error("unsupported native biquad type: " + node.type);
        }
        return [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0];
    }

    function renderAudioNode(node, inputs, length, rate, start) {
        const input = inputs[0] || silenceBus(1, length);
        const channels = input.length;
        if (node instanceof GainNode) {
            const output = silenceBus(channels, length);
            for (let channel = 0; channel < channels; channel++) {
                for (let index = 0; index < length; index++) {
                    output[channel][index] = input[channel][index] * node.gain.__valueAt(start + index / rate);
                }
            }
            return [output];
        }
        if (node instanceof StereoPannerNode) {
            const output = silenceBus(2, length);
            for (let index = 0; index < length; index++) {
                const pan = Math.max(-1, Math.min(1, node.pan.__valueAt(start + index / rate)));
                const mono = channels === 1 ? input[0][index] : 0;
                if (channels === 1) {
                    const angle = (pan + 1) * Math.PI / 4;
                    output[0][index] = mono * Math.cos(angle);
                    output[1][index] = mono * Math.sin(angle);
                } else {
                    const left = input[0][index];
                    const right = input[1][index];
                    output[0][index] = pan <= 0 ? left : left * (1 - pan);
                    output[1][index] = pan >= 0 ? right : right * (1 + pan);
                }
            }
            return [output];
        }
        if (node instanceof BiquadFilterNode) {
            const output = silenceBus(channels, length);
            const states = Array.from({ length: channels }, () => [0, 0, 0, 0]);
            for (let index = 0; index < length; index++) {
                const [b0, b1, b2, a1, a2] = biquadCoefficients(node, start + index / rate, rate);
                for (let channel = 0; channel < channels; channel++) {
                    const state = states[channel];
                    const sample = input[channel][index];
                    const value = b0 * sample + b1 * state[0] + b2 * state[1] - a1 * state[2] - a2 * state[3];
                    state[1] = state[0];
                    state[0] = sample;
                    state[3] = state[2];
                    state[2] = value;
                    output[channel][index] = value;
                }
            }
            return [output];
        }
        if (node instanceof DelayNode) {
            const output = silenceBus(channels, length);
            const maxFrames = Math.max(2, Math.ceil(node.__maxDelayTime * rate) + 2);
            const ring = Array.from({ length: channels }, () => new Float32Array(maxFrames));
            let write = 0;
            for (let index = 0; index < length; index++) {
                const delay = Math.max(0, Math.min(node.__maxDelayTime, node.delayTime.__valueAt(start + index / rate))) * rate;
                for (let channel = 0; channel < channels; channel++) {
                    const state = ring[channel];
                    if (delay <= 0) {
                        output[channel][index] = input[channel][index];
                    } else {
                        const read = write - delay;
                        const wrapped = ((read % maxFrames) + maxFrames) % maxFrames;
                        const lower = Math.floor(wrapped);
                        const upper = (lower + 1) % maxFrames;
                        const fraction = wrapped - lower;
                        output[channel][index] = state[lower] * (1 - fraction) + state[upper] * fraction;
                    }
                    state[write] = input[channel][index];
                }
                write = (write + 1) % maxFrames;
            }
            return [output];
        }
        if (node instanceof WaveShaperNode) {
            const curve = node.__curve;
            if (!curve || curve.length < 2) return [input];
            const output = silenceBus(channels, length);
            for (let channel = 0; channel < channels; channel++) {
                for (let index = 0; index < length; index++) {
                    const position = (Math.max(-1, Math.min(1, input[channel][index])) + 1) * 0.5 * (curve.length - 1);
                    const lower = Math.floor(position);
                    const upper = Math.min(curve.length - 1, lower + 1);
                    const fraction = position - lower;
                    output[channel][index] = curve[lower] * (1 - fraction) + curve[upper] * fraction;
                }
            }
            return [output];
        }
        if (node instanceof DynamicsCompressorNode) {
            const output = silenceBus(channels, length);
            let envelope = 0;
            let reduction = 0;
            for (let index = 0; index < length; index++) {
                const time = start + index / rate;
                let level = 0;
                for (let channel = 0; channel < channels; channel++) level = Math.max(level, Math.abs(input[channel][index]));
                const inputDb = 20 * Math.log10(Math.max(1e-8, level));
                const threshold = node.threshold.__valueAt(time);
                const ratio = Math.max(1, node.ratio.__valueAt(time));
                let target = 0;
                if (inputDb > threshold) target = threshold + (inputDb - threshold) / ratio - inputDb;
                const attack = Math.max(0.0001, node.attack.__valueAt(time));
                const release = Math.max(0.0001, node.release.__valueAt(time));
                const coefficient = target < envelope
                    ? Math.exp(-1 / (attack * rate))
                    : Math.exp(-1 / (release * rate));
                envelope = coefficient * envelope + (1 - coefficient) * target;
                reduction = Math.max(reduction, -envelope);
                const gain = Math.pow(10, envelope / 20);
                for (let channel = 0; channel < channels; channel++) output[channel][index] = input[channel][index] * gain;
            }
            node.reduction = reduction;
            return [output];
        }
        if (node instanceof ConvolverNode) {
            const impulse = node.buffer;
            if (!impulse) return [silenceBus(channels, length)];
            if (impulse.duration > 5) throw new RangeError("native convolver impulse responses are limited to 5 seconds");
            const impulseChannels = Math.min(2, Math.max(1, impulse.numberOfChannels));
            const ir = [];
            let sum = 0;
            for (let channel = 0; channel < impulseChannels; channel++) {
                const data = impulse.getChannelData(channel);
                ir.push(data);
                for (let index = 0; index < data.length; index++) sum += data[index] * data[index];
            }
            const scale = node.normalize && sum > 0
                ? 0.00125 / Math.sqrt(sum / Math.max(1, impulseChannels * impulse.length))
                : 1;
            const output = silenceBus(Math.max(channels, impulseChannels), length);
            for (let channel = 0; channel < output.length; channel++) {
                const inputChannel = input[Math.min(channel, channels - 1)];
                const impulseChannel = ir[Math.min(channel, impulseChannels - 1)];
                for (let index = 0; index < length; index++) {
                    let value = 0;
                    const count = Math.min(index + 1, impulseChannel.length);
                    for (let tap = 0; tap < count; tap++) {
                        value += inputChannel[index - tap] * impulseChannel[tap];
                    }
                    output[channel][index] = value * scale;
                }
            }
            return [output];
        }
        if (node instanceof ChannelSplitterNode) {
            const outputs = [];
            for (let channel = 0; channel < node.numberOfOutputs; channel++) {
                const output = new Float32Array(length);
                if (channel < channels) output.set(input[channel]);
                outputs.push([output]);
            }
            return outputs;
        }
        if (node instanceof ChannelMergerNode) {
            const output = silenceBus(node.numberOfInputs, length);
            for (let channel = 0; channel < node.numberOfInputs; channel++) {
                const bus = inputs[channel] || silenceBus(1, length);
                output[channel].set(bus[0]);
            }
            return [output];
        }
        if (node instanceof IIRFilterNode) {
            const output = silenceBus(channels, length);
            const feedforward = node.__feedforward;
            const feedback = node.__feedback;
            const history = Math.max(feedforward.length, feedback.length, 1);
            for (let channel = 0; channel < channels; channel++) {
                const x = new Float32Array(history);
                const y = new Float32Array(history);
                let position = 0;
                for (let index = 0; index < length; index++) {
                    x[position] = input[channel][index];
                    let value = 0;
                    for (let tap = 0; tap < feedforward.length; tap++) {
                        value += feedforward[tap] * x[(position - tap + history) % history];
                    }
                    for (let tap = 1; tap < feedback.length; tap++) {
                        value -= feedback[tap] * y[(position - tap + history) % history];
                    }
                    value /= feedback[0];
                    y[position] = value;
                    output[channel][index] = value;
                    position = (position + 1) % history;
                }
            }
            return [output];
        }
        if (node instanceof PannerNode) {
            const output = silenceBus(2, length);
            for (let index = 0; index < length; index++) {
                const time = start + index / rate;
                const x = node.positionX.__valueAt(time);
                const y = node.positionY.__valueAt(time);
                const z = node.positionZ.__valueAt(time);
                const distance = Math.sqrt(x * x + y * y + z * z);
                let gain = 1;
                if (node.distanceModel === "linear") {
                    const range = Math.max(1e-6, node.maxDistance - node.refDistance);
                    gain = Math.max(0, 1 - node.rolloffFactor * (distance - node.refDistance) / range);
                } else if (node.distanceModel === "exponential") {
                    gain = Math.pow(Math.max(node.refDistance, distance) / node.refDistance, -node.rolloffFactor);
                } else {
                    gain = node.refDistance / (node.refDistance + node.rolloffFactor * Math.max(0, distance - node.refDistance));
                }
                const pan = Math.max(-1, Math.min(1, x / (Math.abs(x) + Math.abs(z) + 1)));
                const mono = (channels === 1 ? input[0][index] : (input[0][index] + input[1][index]) * 0.5) * gain;
                const angle = (pan + 1) * Math.PI / 4;
                output[0][index] = mono * Math.cos(angle);
                output[1][index] = mono * Math.sin(angle);
            }
            return [output];
        }
        if (node instanceof AnalyserNode) return [input];
        return [input];
    }

    function renderGraph(source, start, duration) {
        if (duration > MAX_RENDER_SECONDS) throw new RangeError("native audio graphs are limited to 30 seconds");
        const rate = source.context.sampleRate;
        const length = Math.max(1, Math.ceil(Math.max(0.001, duration) * rate - 1e-6));
        const order = graphOrder(source);
        const destination = order.find((node) => node.__isDestination);
        if (!destination) return null;
        const incoming = new Map();
        for (const node of order) {
            for (const edge of node.__edges) {
                if (!incoming.has(edge.node)) incoming.set(edge.node, []);
                incoming.get(edge.node).push({ from: node, output: edge.output, input: edge.input });
            }
        }
        const outputs = new Map();
        outputs.set(source, [renderSourceOutput(source, start, length, rate)]);
        for (const node of order) {
            if (node === source || node.__isDestination) continue;
            const edges = incoming.get(node) || [];
            const buses = new Map();
            for (const edge of edges) {
                const from = outputs.get(edge.from);
                if (!from) continue;
                if (!buses.has(edge.input)) buses.set(edge.input, []);
                buses.get(edge.input).push(from[edge.output] || []);
            }
            const count = Math.max(1, ...Array.from(buses.keys()).map((index) => index + 1));
            const inputs = [];
            for (let index = 0; index < count; index++) inputs.push(sumInputBuses(buses.get(index) || [], length));
            outputs.set(node, renderAudioNode(node, inputs, length, rate, start));
        }
        const edges = incoming.get(destination) || [];
        const buses = [];
        for (const edge of edges) {
            const from = outputs.get(edge.from);
            if (from) buses.push(from[edge.output] || []);
        }
        const mixed = sumInputBuses(buses, length, 2);
        const rendered = source.context.createBuffer(2, length, rate);
        rendered.getChannelData(0).set(mixed[0]);
        rendered.getChannelData(1).set(mixed[1]);
        return rendered;
    }

    class OscillatorNode extends AudioBufferSourceNode {
        constructor(context) {
            super(context);
            this.type = "sine";
            this.frequency = new AudioParam(440, this);
        }

        start(when = 0) {
            if (this.__started) throw new Error("cannot call start more than once");
            this.__started = true;
            this.__stopped = false;
            this.__offset = 0;
            this.__startTime = Math.max(audioNow(), Number(when) || 0);
            this.__loop = this.__stopTime < 0;
            setTimeoutFn(() => this.__createVoice(), 0);
        }

        setPeriodicWave(wave) {
            this.__periodicWave = wave;
            this.type = "custom";
        }
    }

    class MediaElementAudioSourceNode extends AudioNode {
        constructor(context, element) {
            super(context);
            this.mediaElement = element;
            element.__source = this;
        }

        connect(node) {
            const result = super.connect(node);
            if (this.mediaElement) this.mediaElement.__syncAudio();
            return result;
        }

        disconnect() {
            super.disconnect();
            if (this.mediaElement) this.mediaElement.__syncAudio();
        }
    }

    function decodeAudio(request, start) {
        return new Promise((resolve, reject) => {
            pendingDecodes.set(request, { resolve, reject });
            try {
                start();
            } catch (error) {
                pendingDecodes.delete(request);
                reject(error);
            }
        });
    }

    function decodeAudioBytes(bytes) {
        if (!audioBridge || typeof audioBridge.decode !== "function") {
            return Promise.reject(new Error("audio decoding is not available"));
        }
        const request = nextAudioRequest++;
        return decodeAudio(request, () => audioBridge.decode(request, bytesView(bytes)));
    }

    function decodeAudioAsset(path) {
        if (!audioBridge || typeof audioBridge.decodeAsset !== "function") {
            return Promise.reject(new Error("audio decoding is not available"));
        }
        const request = nextAudioRequest++;
        return decodeAudio(request, () => audioBridge.decodeAsset(request, String(path)));
    }

    function isBundleAsset(url) {
        return Boolean(url) && !/^[a-z][a-z0-9+.-]*:/i.test(url) && native.fs.size(url) >= 0;
    }

    class HTMLAudioElement {
        constructor(src) {
            this.src = src === undefined ? "" : String(src);
            this.__loop = false;
            this.crossOrigin = null;
            this.onended = null;
            this.muted = false;
            this.paused = true;
            this.duration = NaN;
            this.__volume = 1;
            this.__playbackRate = 1;
            this.__offset = 0;
            this.__voice = -1;
            this.__buffer = null;
            this.__ready = null;
            this.__source = null;
            this.__listeners = makeListeners();
        }

        get loop() {
            return this.__loop;
        }

        set loop(value) {
            this.__loop = Boolean(value);
            this.__syncAudio();
        }

        get volume() {
            return this.__volume;
        }

        set volume(value) {
            this.__volume = Math.min(1, Math.max(0, Number(value) || 0));
            this.__syncAudio();
        }

        get playbackRate() {
            return this.__playbackRate;
        }

        set playbackRate(value) {
            this.__playbackRate = Math.max(0, Number(value) || 0);
            this.__syncAudio();
        }

        get currentTime() {
            if (this.__voice >= 0 && audioBridge && audioBridge.position) {
                const position = audioBridge.position(this.__voice);
                if (position >= 0) return position;
            }
            return this.__offset;
        }

        set currentTime(value) {
            this.__offset = Math.max(0, Number(value) || 0);
            if (this.__voice >= 0) {
                this.__restart();
            }
        }

        play() {
            if (!this.paused && this.__voice >= 0) return Promise.resolve();
            this.paused = false;
            if (this.__buffer) {
                this.__createVoice();
                return Promise.resolve();
            }
            if (!this.__ready) {
                this.__ready = this.__loadSource();
            }
            return this.__ready
                .then(() => {
                    if (!this.paused) this.__createVoice();
                })
                .catch(() => {});
        }

        pause() {
            if (this.__voice >= 0) {
                this.__offset = this.currentTime;
                audioBridge.stop(this.__voice);
                audioVoices.delete(this);
                this.__voice = -1;
            }
            this.paused = true;
        }

        load() {}

        __decode(bytes) {
            return this.__adoptBytes(bytes);
        }

        __adoptBytes(bytes) {
            if (!this.__decodePromise) {
                this.__decodePromise = decodeAudioBytes(bytes).then((buffer) => {
                    this.__buffer = buffer;
                    this.duration = buffer.duration;
                    return buffer;
                });
            }
            return this.__decodePromise;
        }

        __loadSource() {
            if (this.__decodePromise) return this.__decodePromise;
            this.__decodePromise = (isBundleAsset(this.src)
                ? decodeAudioAsset(this.src)
                : fetchResource(this.src)
                    .then((response) => response.arrayBuffer())
                    .then((bytes) => decodeAudioBytes(bytes)))
                .then((buffer) => {
                    this.__buffer = buffer;
                    this.duration = buffer.duration;
                    return buffer;
                });
            return this.__decodePromise;
        }

        __createVoice() {
            if (!audioAvailable()) return;
            const buffer = this.__buffer;
            const nativeId = buffer.__upload();
            if (!nativeId) return;
            const voice = audioBridge.play(
                nativeId,
                this.__gain(),
                0,
                this.__playbackRate,
                this.loop,
                this.__offset,
                -1,
            );
            if (voice < 0) return;
            this.__voice = voice;
            audioVoices.add(this);
        }

        __restart() {
            if (this.__voice >= 0) {
                audioBridge.stop(this.__voice);
                audioVoices.delete(this);
                this.__voice = -1;
            }
            if (!this.paused) this.__createVoice();
        }

        __gain() {
            if (this.muted || !this.__source) return this.muted ? 0 : this.__volume;
            const params = graphParams(this.__source);
            return params.gain * this.__volume;
        }

        __syncAudio() {
            if (this.__voice < 0) return;
            audioBridge.setGain(this.__voice, this.__gain());
            audioBridge.setRate(this.__voice, this.__playbackRate);
            audioBridge.setLoop(this.__voice, this.loop);
        }

        __fireEnded() {
            this.paused = true;
            this.__offset = this.buffer ? this.buffer.duration : this.__offset;
            this.__voice = -1;
            const event = makeEvent("ended", {});
            event.target = this;
            try {
                if (typeof this.onended === "function") this.onended(event);
            } catch (error) {
                reportError(error);
            }
            this.__listeners.emit(event);
        }

        addEventListener(type, fn) {
            this.__listeners.add(type, fn);
        }

        removeEventListener(type, fn) {
            this.__listeners.remove(type, fn);
        }
    }

    class HTMLVideoElement {
        constructor(src) {
            this.__src = src === undefined ? "" : String(src);
            this.crossOrigin = null;
            this.playsInline = false;
            this.autoplay = false;
            this.preload = "auto";
            this.width = 0;
            this.height = 0;
            this.onended = null;
            this.error = null;
            this.paused = true;
            this.ended = false;
            this.duration = NaN;
            this.videoWidth = 0;
            this.videoHeight = 0;
            this.__networkState = 0;
            this.__loop = false;
            this.__volume = 1;
            this.__muted = false;
            this.__playbackRate = 1;
            this.__base = 0;
            this.__startedAt = 0;
            this.__nativeId = 0;
            this.__generation = 0;
            this.__ready = null;
            this.__timer = 0;
            this.__readyTimer = 0;
            this.__dataFired = false;
            this.__seekPending = false;
            this.__listeners = makeListeners();
            this.__audio = new HTMLAudioElement(this.__src);
        }

        get src() {
            return this.__src;
        }

        get currentSrc() {
            return this.__src;
        }

        get networkState() {
            return this.__networkState;
        }

        get readyState() {
            if (!this.__nativeId) return 0;
            if (this.ended) return 2;
            return native.video.ready(this.__nativeId, this.currentTime) ? 2 : 1;
        }

        set src(value) {
            const url = value === undefined ? "" : String(value);
            if (url === this.__src) return;
            this.__src = url;
            this.load();
        }

        get loop() {
            return this.__loop;
        }

        set loop(value) {
            this.__loop = Boolean(value);
            this.__audio.loop = this.__loop;
        }

        get volume() {
            return this.__volume;
        }

        set volume(value) {
            this.__volume = Math.min(1, Math.max(0, Number(value) || 0));
            this.__audio.volume = this.__volume;
        }

        get muted() {
            return this.__muted;
        }

        set muted(value) {
            this.__muted = Boolean(value);
            this.__audio.muted = this.__muted;
        }

        get playbackRate() {
            return this.__playbackRate;
        }

        set playbackRate(value) {
            this.__playbackRate = Math.max(0, Number(value) || 0);
            this.__audio.playbackRate = this.__playbackRate;
        }

        get currentTime() {
            if (this.paused) return this.__base;
            const elapsed = (audioNow() - this.__startedAt) * this.__playbackRate;
            let time = this.__base + elapsed;
            if (this.__loop && Number.isFinite(this.duration) && this.duration > 0) {
                time %= this.duration;
            } else if (Number.isFinite(this.duration) && time > this.duration) {
                time = this.duration;
            }
            return time;
        }

        set currentTime(value) {
            const time = Math.max(0, Number(value) || 0);
            this.__base = Number.isFinite(this.duration) && this.duration > 0
                ? Math.min(time, this.duration)
                : time;
            this.__startedAt = audioNow();
            this.ended = false;
            this.__audio.currentTime = this.__base;
            this.__seekPending = true;
            this.__fire("seeking");
            this.__startReadyTimer();
        }

        play() {
            if (!this.paused) return Promise.resolve();
            this.paused = false;
            this.ended = false;
            this.__startedAt = audioNow();
            const ready = this.__prepare();
            ready
                .then(() => {
                    if (this.paused) return;
                    this.__audio.currentTime = this.__base;
                    this.__audio.play().catch(() => {});
                    this.__fire("playing");
                    this.__startTicking();
                })
                .catch((error) => {
                    reportError(error);
                    this.paused = true;
                    this.__stopTicking();
                    this.__fire("error");
                });
            return ready.then(() => {});
        }

        pause() {
            if (this.paused) return;
            this.__base = this.currentTime;
            this.paused = true;
            this.ended = false;
            this.__stopTicking();
            this.__audio.pause();
        }

        load() {
            this.__generation++;
            if (this.__nativeId) {
                native.video.close(this.__nativeId);
                this.__nativeId = 0;
            }
            this.__audio.pause();
            this.__ready = null;
            this.__base = 0;
            this.__startedAt = 0;
            this.paused = true;
            this.ended = false;
            this.duration = NaN;
            this.videoWidth = 0;
            this.videoHeight = 0;
            this.error = null;
            this.__networkState = this.__src ? 1 : 0;
            this.__dataFired = false;
            this.__seekPending = false;
            this.__stopTicking();
            this.__stopReadyTimer();
            this.__audio = new HTMLAudioElement(this.__src);
        }

        canPlayType(type) {
            return /mp4|m4v|avc1/i.test(String(type)) ? "probably" : "";
        }

        addEventListener(type, fn) {
            this.__listeners.add(type, fn);
        }

        removeEventListener(type, fn) {
            this.__listeners.remove(type, fn);
        }

        __prepare() {
            if (this.__ready) return this.__ready;
            if (!this.__src) {
                this.__ready = Promise.reject(new Error("no video source"));
                return this.__ready;
            }
            const url = this.__src;
            const generation = this.__generation;
            this.__networkState = 2;
            this.__ready = this.__open(url, generation)
                .then((info) => {
                    if (generation !== this.__generation || this.__src !== url) {
                        native.video.close(info.id);
                        throw new Error("video source changed");
                    }
                    this.__nativeId = info.id;
                    this.videoWidth = info.width;
                    this.videoHeight = info.height;
                    this.duration = info.duration > 0 ? info.duration : NaN;
                    this.__networkState = 1;
                    this.__dataFired = false;
                    this.__fire("loadedmetadata");
                    this.__startReadyTimer();
                    return info;
                })
                .catch((error) => {
                    if (generation !== this.__generation || this.__src !== url) throw error;
                    this.__networkState = 3;
                    this.error = { code: 4, message: String((error && error.message) || error) };
                    throw error;
                });
            return this.__ready;
        }

        __open(url, generation) {
            if (!native.video) return Promise.reject(new Error("video playback is not available"));
            const relative = isBundleAsset(url);
            if (relative && typeof native.video.openAsset === "function") {
                const info = native.video.openAsset(url);
                if (info) return Promise.resolve(info);
            }
            const audio = this.__audio;
            return fetchResource(url)
                .then((response) => response.arrayBuffer())
                .then((bytes) => {
                    const info = native.video.open(bytesView(bytes));
                    if (!info) throw new Error("unsupported video: " + url);
                    if (generation === this.__generation && audio === this.__audio) {
                        audio.__adoptBytes(bytes).catch(() => {});
                    }
                    return info;
                });
        }

        __startReadyTimer() {
            if (this.__readyTimer) return;
            this.__checkFrame();
            if (this.__readyTimer || (this.__dataFired && !this.__seekPending)) return;
            let attempts = 0;
            this.__readyTimer = setIntervalFn(() => {
                if (++attempts > 100) {
                    this.__stopReadyTimer();
                    this.error = { code: 4, message: "video frame did not become ready" };
                    this.__fire("error");
                    return;
                }
                this.__checkFrame();
            }, 100);
        }

        __stopReadyTimer() {
            if (!this.__readyTimer) return;
            clearTimer(this.__readyTimer);
            this.__readyTimer = 0;
        }

        __checkFrame() {
            if (!this.__nativeId) return;
            if (!this.__dataFired && this.readyState >= 2) {
                this.__dataFired = true;
                this.__fire("loadeddata");
                this.__fire("canplay");
            }
            if (this.__seekPending && this.readyState >= 2) {
                this.__seekPending = false;
                this.__fire("timeupdate");
                this.__fire("seeked");
            }
            if (this.__dataFired && !this.__seekPending) this.__stopReadyTimer();
        }

        __startTicking() {
            if (this.__timer) return;
            this.__tick();
            this.__timer = setIntervalFn(() => this.__tick(), 250);
        }

        __stopTicking() {
            if (!this.__timer) return;
            clearTimer(this.__timer);
            this.__timer = 0;
        }

        __tick() {
            if (this.paused) return;
            this.__checkFrame();
            this.__fire("timeupdate");
            if (
                !this.__loop
                && Number.isFinite(this.duration)
                && this.currentTime >= this.duration
            ) {
                this.__base = this.duration;
                this.paused = true;
                this.ended = true;
                this.__stopTicking();
                this.__audio.pause();
                this.__fire("ended");
            }
        }

        __fire(type) {
            const event = makeEvent(type, {});
            event.target = this;
            try {
                const handler = this["on" + type];
                if (typeof handler === "function") handler(event);
            } catch (error) {
                reportError(error);
            }
            this.__listeners.emit(event);
        }
    }

    class AudioContext {
        constructor() {
            this.sampleRate = audioBridge && audioBridge.sampleRate ? audioBridge.sampleRate() || 48000 : 48000;
            this.state = "running";
            this.destination = new AudioDestinationNode(this);
            this.listener = {};
        }

        get currentTime() {
            return audioNow();
        }

        createGain() {
            return new GainNode(this);
        }

        createBufferSource() {
            return new AudioBufferSourceNode(this);
        }

        createStereoPanner() {
            return new StereoPannerNode(this);
        }

        createOscillator() {
            return new OscillatorNode(this);
        }

        createBiquadFilter() {
            return new BiquadFilterNode(this);
        }

        createDelay(maxDelayTime = 1) {
            return new DelayNode(this, maxDelayTime);
        }

        createWaveShaper() {
            return new WaveShaperNode(this);
        }

        createDynamicsCompressor() {
            return new DynamicsCompressorNode(this);
        }

        createConvolver() {
            return new ConvolverNode(this);
        }

        createChannelSplitter(numberOfOutputs = 6) {
            return new ChannelSplitterNode(this, numberOfOutputs);
        }

        createChannelMerger(numberOfInputs = 6) {
            return new ChannelMergerNode(this, numberOfInputs);
        }

        createIIRFilter(feedforward, feedback) {
            return new IIRFilterNode(this, feedforward, feedback);
        }

        createPanner() {
            return new PannerNode(this);
        }

        createConstantSource() {
            return new ConstantSourceNode(this);
        }

        createAnalyser() {
            return new AnalyserNode(this);
        }

        createPeriodicWave(real, imag, constraints) {
            return new PeriodicWave(real, imag, constraints);
        }

        createMediaElementSource(element) {
            return new MediaElementAudioSourceNode(this, element);
        }

        createBuffer(channels, length, sampleRate) {
            return new AudioBuffer({
                numberOfChannels: channels,
                length,
                sampleRate: sampleRate || this.sampleRate,
            });
        }

        decodeAudioData(buffer, success, failure) {
            const promise = new Promise((resolve, reject) => {
                if (!audioBridge || typeof audioBridge.decode !== "function") {
                    reject(new Error("audio decoding is not available"));
                    return;
                }
                const request = nextAudioRequest++;
                pendingDecodes.set(request, {
                    resolve: (audioBuffer) => resolve(audioBuffer),
                    reject,
                });
                audioBridge.decode(request, bytesView(buffer));
            });
            if (typeof success === "function" || typeof failure === "function") {
                promise.then(
                    typeof success === "function" ? success : () => {},
                    typeof failure === "function" ? failure : () => {},
                );
            }
            return promise;
        }

        suspend() {
            this.state = "suspended";
            if (audioBridge && audioBridge.setPaused) audioBridge.setPaused(true);
            return Promise.resolve();
        }

        resume() {
            this.state = "running";
            if (audioBridge && audioBridge.setPaused) audioBridge.setPaused(false);
            return Promise.resolve();
        }

        close() {
            this.state = "closed";
            return Promise.resolve();
        }
    }

    global.__kapNative_onAudioDecoded = function (request, buffer, ok, message) {
        const pending = pendingDecodes.get(request);
        if (!pending) return;
        pendingDecodes.delete(request);
        if (!ok) {
            pending.reject(new Error(message || "failed to decode audio"));
            return;
        }
        const info = new Float32Array(3);
        let sampleRate = 48000;
        let channels = 1;
        let length = 0;
        if (audioBridge && typeof audioBridge.bufferInfo === "function" && audioBridge.bufferInfo(buffer, info)) {
            sampleRate = info[0] || 48000;
            channels = info[1] || 1;
            length = info[2] || 0;
        }
        pending.resolve(
            new AudioBuffer({
                numberOfChannels: channels,
                length,
                sampleRate,
                __nativeId: buffer,
            }),
        );
    };

    global.__kapNative_onVoiceEnded = function (voice) {
        for (const node of audioVoices) {
            if (node.__voice === voice) {
                audioVoices.delete(node);
                node.__voice = -1;
                node.__fireEnded();
                return;
            }
        }
    };

    // -------------------------------------------------------------- observables

    const resizeObservers = new Set();

    class ResizeObserver {
        constructor(callback) {
            this.callback = callback;
            this.targets = new Set();
            resizeObservers.add(this);
        }

        observe(target) {
            this.targets.add(target);
        }

        unobserve(target) {
            this.targets.delete(target);
        }

        disconnect() {
            this.targets.clear();
            resizeObservers.delete(this);
        }

        __notify() {
            const entries = [];
            for (const target of this.targets) {
                entries.push({
                    target,
                    contentRect: target.getBoundingClientRect(),
                });
            }
            if (entries.length) {
                try {
                    this.callback(entries, this);
                } catch (error) {
                    reportError(error);
                }
            }
        }
    }

    function notifyResize() {
        for (const observer of Array.from(resizeObservers)) observer.__notify();
    }

    class MutationObserver {
        constructor(callback) {
            this.callback = callback;
            this.targets = new Set();
            this.records = [];
        }

        observe(target) {
            this.targets.add(target);
        }

        disconnect() {
            this.targets.clear();
            this.records = [];
        }

        takeRecords() {
            const records = this.records;
            this.records = [];
            return records;
        }
    }

    // --------------------------------------------------------------- dom events

    function makeEvent(type, payload) {
        payload = payload || {};
        const event = {
            type,
            target: null,
            defaultPrevented: false,
            preventDefault() {
                this.defaultPrevented = true;
            },
            stopPropagation() {},
            stopImmediatePropagation() {},
            getModifierState(name) {
                if (name === "CapsLock") return Boolean(payload.capsLock);
                if (name === "Shift") return Boolean(payload.shiftKey);
                if (name === "Control") return Boolean(payload.ctrlKey);
                if (name === "Alt") return Boolean(payload.altKey);
                if (name === "Meta") return Boolean(payload.metaKey);
                return false;
            },
        };
        return event;
    }

    function canvasCoordinates(clientX, clientY) {
        const rect = mainCanvas ? canvasSize(mainCanvas) : { x: 0, y: 0, scale: 1 };
        if (!rect.scale) return { x: 0, y: 0 };
        return {
            x: (clientX - rect.x) / rect.scale,
            y: (clientY - rect.y) / rect.scale,
        };
    }

    function dispatchPointer(type, payload) {
        const event = makeEvent(type, payload);
        Object.assign(event, payload);
        const position = canvasCoordinates(payload.clientX || 0, payload.clientY || 0);
        event.offsetX = position.x;
        event.offsetY = position.y;
        event.movementX = payload.movementX || 0;
        event.movementY = payload.movementY || 0;
        event.buttons = mouseButtons;
        if (type === "mousedown" || type === "mouseup") {
            const target = textControlAt(position.x, position.y);
            if (target) {
                event.target = target;
                if (type === "mousedown") target.focus();
                target.__emit(type, event);
                if (type === "mouseup") {
                    const click = makeEvent("click", {});
                    click.target = target;
                    target.__emit("click", click);
                }
                documentListeners.emit(event);
                windowListeners.emit(event);
                return;
            }
        }
        if (type === "mousedown" && isTextControl(document.activeElement)) document.activeElement.blur();
        canvasListeners.emit(event);
    }

    function isTextControl(element) {
        return element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement;
    }

    function styleLength(value) {
        if (typeof value === "number") return value;
        if (typeof value === "string" && value.trim().endsWith("px")) return parseFloat(value) || 0;
        return null;
    }

    function textControlAt(x, y) {
        let hit = null;
        for (const element of createdElements) {
            if (!isTextControl(element) || !isAttached(element)) continue;
            const style = element.__style;
            if (style.display === "none" || style.visibility === "hidden" || style.pointerEvents === "none") continue;
            if (style.position !== "absolute" && style.position !== "fixed") continue;
            const zIndex = Number(style.zIndex);
            if (Number.isFinite(zIndex) && zIndex < 0) continue;
            const left = styleLength(style.left);
            const top = styleLength(style.top);
            const width = styleLength(style.width);
            const height = styleLength(style.height);
            if (left === null || top === null || width === null || height === null) continue;
            if (x >= left && x <= left + width && y >= top && y <= top + height) hit = element;
        }
        return hit;
    }

    function editTextValue(element, event) {
        const key = event.key;
        if (!key || event.ctrlKey || event.metaKey || event.altKey) return;
        let value = String(element.value);
        if (key === "Backspace") {
            value = value.slice(0, -1);
        } else if (key === "Enter") {
            if (element.tagName !== "TEXTAREA") return;
            value += "\n";
        } else if (key.length === 1) {
            const max = Number(element.maxLength);
            if (max >= 0 && value.length >= max) return;
            value += key;
        } else {
            return;
        }
        if (value === element.value) return;
        element.value = value;
        const inputEvent = makeEvent("input", {});
        inputEvent.target = element;
        element.__emit("input", inputEvent);
    }

    function dispatchKey(type, payload) {
        const event = makeEvent(type, payload);
        Object.assign(event, payload);
        const active = isTextControl(document.activeElement) ? document.activeElement : null;
        if (active) {
            event.target = active;
            active.__emit(type, event);
            if (type === "keydown" && !event.defaultPrevented) editTextValue(active, event);
            documentListeners.emit(event);
            windowListeners.emit(event);
            return;
        }
        canvasListeners.emit(event);
    }

    function dispatchWheel(payload) {
        const event = makeEvent("wheel", payload);
        event.deltaX = payload.deltaX || 0;
        event.deltaY = payload.deltaY || 0;
        event.deltaMode = 0;
        canvasListeners.emit(event);
    }

    let mouseButtons = 0;

    global.__kapNative_onFrame = function (now) {
        inFrame = true;
        frameTime = now;
        tickTimers(now);
        const queue = frames;
        frames = [];
        for (const entry of queue) {
            try {
                entry.callback(now);
            } catch (error) {
                reportError(error);
            }
        }
        audioTick();
        inFrame = false;
    };

    global.__kapNative_dispatch = function (type, payloadJson) {
        let payload = {};
        try {
            payload = JSON.parse(payloadJson);
        } catch (error) {
            reportError(error);
        }
        switch (type) {
            case "resize": {
                screen.width = payload.width;
                screen.height = payload.height;
                screen.dpr = payload.dpr;
                notifyResize();
                windowListeners.emit(makeEvent("resize", payload));
                break;
            }
            case "visibilitychange": {
                document.visibilityState = payload.hidden ? "hidden" : "visible";
                documentListeners.emit(makeEvent("visibilitychange", payload));
                break;
            }
            case "focus":
                windowListeners.emit(makeEvent("focus", payload));
                break;
            case "blur":
                windowListeners.emit(makeEvent("blur", payload));
                break;
            case "keydown":
                dispatchKey("keydown", payload);
                break;
            case "keyup":
                dispatchKey("keyup", payload);
                break;
            case "mousemove":
                dispatchPointer("mousemove", payload);
                break;
            case "mousedown":
                mouseButtons |= 1 << payload.button;
                dispatchPointer("mousedown", payload);
                break;
            case "mouseup":
                mouseButtons &= ~(1 << payload.button);
                dispatchPointer("mouseup", payload);
                break;
            case "wheel":
                dispatchWheel(payload);
                break;
            case "gamepadconnected": {
                const event = makeEvent("gamepadconnected", payload);
                event.gamepad = gamepadFromSnapshot(payload);
                windowListeners.emit(event);
                break;
            }
            case "gamepaddisconnected": {
                const event = makeEvent("gamepaddisconnected", payload);
                event.gamepad = disconnectGamepad(payload);
                windowListeners.emit(event);
                break;
            }
            default:
                break;
        }
    };

    // -------------------------------------------------------------------- dom

    const createdElements = new Set();

    function registerTree(element) {
        if (!element || typeof element !== "object") return;
        if (
            typeof element.tagName === "string"
            && element !== documentElement
            && element !== head
            && element !== body
        ) {
            createdElements.add(element);
        }
        if (Array.isArray(element.children)) {
            for (const child of element.children) registerTree(child);
        }
    }

    function unregisterTree(element) {
        if (!element || typeof element !== "object") return;
        createdElements.delete(element);
        if (Array.isArray(element.children)) {
            for (const child of element.children) unregisterTree(child);
        }
    }

    function isAttached(element) {
        let node = element;
        while (node) {
            if (node === body || node === documentElement || node === head) return true;
            node = node instanceof HTMLCanvasElement ? node.__parent : node.parentElement;
        }
        return false;
    }

    class TextNode {
        constructor(text) {
            this.nodeType = 3;
            this.textContent = String(text);
        }
    }

    class HTMLElement {
        constructor(tagName) {
            this.tagName = String(tagName).toUpperCase();
            this.nodeName = this.tagName;
            this.id = "";
            this.className = "";
            this.textContent = "";
            this.innerHTML = "";
            this.value = "";
            this.checked = false;
            this.disabled = false;
            this.type = "";
            this.name = "";
            this.placeholder = "";
            this.maxLength = -1;
            this.pattern = "";
            this.noValidate = false;
            this.isConnected = true;
            this.children = [];
            this.parentElement = null;
            this.__style = makeStyle();
            this.__listeners = new Map();
            this.__attributes = new Map();
        }

        get style() {
            return this.__style;
        }

        set style(value) {
            if (typeof value === "string") this.__style.cssText = value;
        }

        setAttribute(name, value) {
            const key = String(name);
            this.__attributes.set(key, String(value));
            if (key === "id") this.id = String(value);
            if (key === "class") this.className = String(value);
            if (["src", "href", "download", "rel", "type", "name", "value"].includes(key)) this[key] = String(value);
        }

        getAttribute(name) {
            const key = String(name);
            return this.__attributes.has(key) ? this.__attributes.get(key) : null;
        }

        hasAttribute(name) {
            return this.__attributes.has(String(name));
        }

        get classList() {
            const element = this;
            const names = () => new Set(element.className.split(/\s+/).filter(Boolean));
            return {
                add(...values) {
                    element.className = [...new Set([...names(), ...values])].join(" ");
                },
                remove(...values) {
                    element.className = [...names()].filter((value) => !values.includes(value)).join(" ");
                },
                contains(value) {
                    return names().has(value);
                },
                toggle(value, force) {
                    const enabled = force === undefined ? !this.contains(value) : Boolean(force);
                    if (enabled) this.add(value);
                    else this.remove(value);
                    return enabled;
                },
            };
        }

        removeAttribute(name) {
            this.__attributes.delete(String(name));
        }

        __adopt(child) {
            if (!child || typeof child !== "object") return;
            const current = child instanceof HTMLCanvasElement ? child.__parent : child.parentElement;
            if (current && typeof current.removeChild === "function") current.removeChild(child);
            if (child instanceof HTMLCanvasElement) {
                child.__parent = this;
                if (!mainCanvas || this.tagName === "BODY") mainCanvas = child;
            } else {
                child.parentElement = this;
            }
            registerTree(child);
        }

        appendChild(child) {
            if (!child) return child;
            this.__adopt(child);
            this.children.push(child);
            return child;
        }

        append(...nodes) {
            for (const node of nodes) {
                this.appendChild(node instanceof TextNode || typeof node !== "string" ? node : new TextNode(node));
            }
        }

        prepend(...nodes) {
            const added = [];
            for (const node of nodes) {
                if (!node) continue;
                this.__adopt(node);
                added.push(node);
            }
            this.children.unshift(...added);
        }

        insertBefore(child, reference) {
            if (!child) return child;
            const index = reference ? this.children.indexOf(reference) : -1;
            this.__adopt(child);
            if (index < 0) this.children.push(child);
            else this.children.splice(index, 0, child);
            return child;
        }

        removeChild(child) {
            const index = this.children.indexOf(child);
            if (index >= 0) this.children.splice(index, 1);
            unregisterTree(child);
            if (child === mainCanvas) mainCanvas = null;
            if (child instanceof HTMLCanvasElement) child.__parent = null;
            else if (child) child.parentElement = null;
            if (child instanceof HTMLElement && (document.activeElement === child || child.contains(document.activeElement))) {
                document.activeElement = null;
            }
            return child;
        }

        remove() {
            if (this.parentElement) this.parentElement.removeChild(this);
            else if (document.activeElement === this) document.activeElement = null;
        }

        contains(node) {
            return this.children.includes(node);
        }

        focus() {
            document.activeElement = this;
        }

        blur() {
            if (document.activeElement === this) document.activeElement = null;
        }

        select() {}

        click() {
            if (this.tagName === "A" && ("download" in this || this.hasAttribute("download"))) {
                const name = this.download === true ? "" : String(this.download || "");
                const href = String(this.href || "");
                if (href) {
                    const data = decodeDataURL(href);
                    if (data) {
                        native.fs.download(name, data);
                        return;
                    }
                    if (/^blob:/i.test(href)) {
                        const blob = objectURLs.get(href);
                        if (blob) {
                            native.fs.download(name, blobBytes(blob));
                            return;
                        }
                    }
                    if (/^https?:/i.test(href) || !/^[a-z][a-z0-9+.-]*:/i.test(href)) {
                        fetchResource(href)
                            .then((response) => response.arrayBuffer())
                            .then((buffer) => native.fs.download(name, new Uint8Array(buffer)))
                            .catch((error) => console.error("download failed:", error));
                        return;
                    }
                }
            }
            this.__emit("click", makeEvent("click", {}));
        }

        getBoundingClientRect() {
            return {
                x: 0,
                y: 0,
                left: 0,
                top: 0,
                right: screen.width,
                bottom: screen.height,
                width: screen.width,
                height: screen.height,
            };
        }

        addEventListener(type, fn) {
            const key = String(type);
            const list = this.__listeners.get(key);
            if (list) list.push(fn);
            else this.__listeners.set(key, [fn]);
        }

        removeEventListener(type, fn) {
            const list = this.__listeners.get(String(type));
            if (!list) return;
            const index = list.indexOf(fn);
            if (index >= 0) list.splice(index, 1);
        }

        dispatchEvent(event) {
            this.__emit(event && event.type ? event.type : String(event), event);
            return true;
        }

        __emit(type, event) {
            const list = this.__listeners.get(String(type));
            if (!list) return;
            for (const fn of list.slice()) {
                try {
                    fn.call(this, event);
                } catch (error) {
                    reportError(error);
                }
            }
        }
    }

    class HTMLInputElement extends HTMLElement {}
    class HTMLTextAreaElement extends HTMLElement {}
    class HTMLFormElement extends HTMLElement {}

    class Event {
        constructor(type, init) {
            const options = init || {};
            this.type = String(type);
            this.bubbles = Boolean(options.bubbles);
            this.cancelable = Boolean(options.cancelable);
            this.defaultPrevented = false;
            this.target = null;
            this.currentTarget = null;
        }

        preventDefault() {
            this.defaultPrevented = true;
        }

        stopPropagation() {}

        stopImmediatePropagation() {}
    }

    class MouseEvent extends Event {
        constructor(type, init) {
            super(type, init);
            const options = init || {};
            this.clientX = Number(options.clientX || 0);
            this.clientY = Number(options.clientY || 0);
            this.screenX = Number(options.screenX || 0);
            this.screenY = Number(options.screenY || 0);
            this.movementX = Number(options.movementX || 0);
            this.movementY = Number(options.movementY || 0);
            this.button = Number(options.button || 0);
            this.buttons = Number(options.buttons || 0);
            this.offsetX = Number(options.offsetX || 0);
            this.offsetY = Number(options.offsetY || 0);
        }
    }

    class KeyboardEvent extends Event {
        constructor(type, init) {
            super(type, init);
            const options = init || {};
            this.key = options.key === undefined ? "" : String(options.key);
            this.code = options.code === undefined ? "" : String(options.code);
            this.repeat = Boolean(options.repeat);
            this.ctrlKey = Boolean(options.ctrlKey);
            this.shiftKey = Boolean(options.shiftKey);
            this.altKey = Boolean(options.altKey);
            this.metaKey = Boolean(options.metaKey);
        }
    }

    class WheelEvent extends MouseEvent {
        constructor(type, init) {
            super(type, init);
            const options = init || {};
            this.deltaX = Number(options.deltaX || 0);
            this.deltaY = Number(options.deltaY || 0);
            this.deltaZ = Number(options.deltaZ || 0);
            this.deltaMode = Number(options.deltaMode || 0);
        }
    }

    class CustomEvent extends Event {
        constructor(type, init) {
            super(type, init);
            this.detail = init ? init.detail : undefined;
        }
    }

    const documentElement = new HTMLElement("html");
    const body = new HTMLElement("body");
    const head = new HTMLElement("head");

    Object.defineProperty(body, "offsetWidth", {
        get: () => screen.width,
        configurable: true,
    });
    Object.defineProperty(body, "offsetHeight", {
        get: () => screen.height,
        configurable: true,
    });

    let mainCanvas = null;

    function utf8Encode(value) {
        const text = String(value);
        const bytes = [];
        for (let i = 0; i < text.length; i++) {
            let code = text.codePointAt(i);
            if (code > 0xffff) i++;
            if (code < 0x80) bytes.push(code);
            else if (code < 0x800) {
                bytes.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f));
            } else if (code < 0x10000) {
                bytes.push(
                    0xe0 | (code >> 12),
                    0x80 | ((code >> 6) & 0x3f),
                    0x80 | (code & 0x3f),
                );
            } else {
                bytes.push(
                    0xf0 | (code >> 18),
                    0x80 | ((code >> 12) & 0x3f),
                    0x80 | ((code >> 6) & 0x3f),
                    0x80 | (code & 0x3f),
                );
            }
        }
        return new Uint8Array(bytes);
    }

    function utf8Decode(value) {
        const bytes = bytesView(value);
        let output = "";
        for (let i = 0; i < bytes.length;) {
            const first = bytes[i++];
            let code;
            if (first < 0x80) code = first;
            else if (first < 0xe0) code = ((first & 0x1f) << 6) | (bytes[i++] & 0x3f);
            else if (first < 0xf0) {
                code = ((first & 0x0f) << 12) | ((bytes[i++] & 0x3f) << 6) | (bytes[i++] & 0x3f);
            } else {
                code = ((first & 0x07) << 18) | ((bytes[i++] & 0x3f) << 12)
                    | ((bytes[i++] & 0x3f) << 6) | (bytes[i++] & 0x3f);
            }
            output += String.fromCodePoint(code);
        }
        return output;
    }

    class TextEncoder {
        encode(value) {
            return utf8Encode(value === undefined ? "" : value);
        }
    }

    class TextDecoder {
        constructor(encoding) {
            this.encoding = encoding || "utf-8";
        }

        decode(value) {
            if (value === undefined) return "";
            return utf8Decode(value);
        }
    }

    const document = {
        body,
        head,
        documentElement,
        title: "",
        visibilityState: "visible",
        activeElement: null,
        fullscreenElement: null,
        pointerLockElement: null,
        fonts: {
            add(face) {
                fontFaces.add(face);
            },
            delete(face) {
                fontFaces.delete(face);
            },
            check(spec) {
                const family = parseFontFamily(spec);
                return native.font.has(family);
            },
            load() {
                return Promise.resolve([]);
            },
            ready: Promise.resolve(),
            forEach(callback) {
                fontFaces.forEach(callback);
            },
        },
        createElement(tag) {
            const name = String(tag).toLowerCase();
            if (name === "canvas") return new HTMLCanvasElement();
            if (name === "video") return new HTMLVideoElement();
            if (name === "audio") return new HTMLAudioElement();
            if (name === "input") return new HTMLInputElement(name);
            if (name === "textarea") return new HTMLTextAreaElement(name);
            if (name === "form") return new HTMLFormElement(name);
            const element = new HTMLElement(name);
            if (name === "link") element.relList = { supports: (value) => value === "modulepreload" };
            return element;
        },
        createElementNS(_namespace, tag) {
            return document.createElement(tag);
        },
        createTextNode(text) {
            return new TextNode(text);
        },
        getElementById(id) {
            const key = String(id);
            for (const element of createdElements) {
                if (element.id === key && isAttached(element)) return element;
            }
            return null;
        },
        getElementsByTagName(name) {
            const tag = String(name).toLowerCase();
            const matches = [];
            for (const element of createdElements) {
                if (element.tagName.toLowerCase() === tag && isAttached(element)) matches.push(element);
            }
            if (tag === "canvas" && mainCanvas) {
                return [mainCanvas].concat(matches.filter((element) => element !== mainCanvas));
            }
            return matches;
        },
        getElementsByClassName(name) {
            const wanted = String(name);
            return Array.from(createdElements).filter((element) =>
                isAttached(element)
                && String(element.className).split(/\s+/).includes(wanted),
            );
        },
        querySelector(selector) {
            return document.querySelectorAll(selector)[0] || null;
        },
        querySelectorAll(selector) {
            const value = String(selector).trim();
            if (!value) return [];
            const match = /^([\w-]+)?(?:#([\w-]+))?(?:\.([\w-]+))?(?:\[([\w-]+)(?:=["']?([^"'\]]+)["']?)?\])?$/.exec(value);
            if (!match) return [];
            const [, tag, id, className, attr, attrValue] = match;
            return [documentElement, head, body, ...createdElements].filter((element) => {
                if (!isAttached(element)) return false;
                return (!tag || element.tagName.toLowerCase() === tag.toLowerCase())
                    && (!id || element.id === id)
                    && (!className || String(element.className).split(/\s+/).includes(className))
                    && (!attr || (element.hasAttribute && element.hasAttribute(attr)
                        && (attrValue === undefined || element.getAttribute(attr) === attrValue)));
            });
        },
        get webkitFullscreenElement() {
            return this.fullscreenElement;
        },
        exitPointerLock() {
            native.window.setCursorLocked(false);
            document.pointerLockElement = null;
            documentListeners.emit(makeEvent("pointerlockchange", {}));
        },
        webkitExitPointerLock() {
            this.exitPointerLock();
        },
        exitFullscreen() {
            native.window.setFullscreen(false);
            document.fullscreenElement = null;
            documentListeners.emit(makeEvent("fullscreenchange", {}));
        },
        webkitExitFullScreen() {
            this.exitFullscreen();
        },
        addEventListener(type, fn) {
            documentListeners.add(type, fn);
        },
        removeEventListener(type, fn) {
            documentListeners.remove(type, fn);
        },
        dispatchEvent(event) {
            documentListeners.emit(event);
            return true;
        },
    };

    global.__kapNative_loadDocument = function (html) {
        for (const element of [head, body]) {
            for (const child of element.children.slice()) element.removeChild(child);
        }
        mainCanvas = null;
        documentElement.children = [];
        documentElement.append(head, body);
        const stack = [body];
        const voidTags = new Set(["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"]);
        const source = String(html).replace(/<!--[\s\S]*?-->/g, "")
            .replace(/<(script|style)\b[^>]*>[\s\S]*?<\/\1\s*>/gi, "");
        const decode = (value) => value.replace(/&(?:quot|apos|amp|lt|gt);/g, (entity) => ({
            "&quot;": '"',
            "&apos;": "'",
            "&amp;": "&",
            "&lt;": "<",
            "&gt;": ">",
        })[entity]);
        const tokens = /<\/?([\w-]+)\b((?:"[^"]*"|'[^']*'|[^'">])*)>|([^<]+)/g;
        for (const token of source.matchAll(tokens)) {
            if (!token[1]) {
                const text = decode(token[3] || "");
                stack[stack.length - 1].textContent += text;
                if (stack[stack.length - 1].tagName === "TITLE") document.title = text;
                continue;
            }
            const tag = token[1].toLowerCase();
            if (token[0].startsWith("</")) {
                const index = stack.map((element) => element.tagName.toLowerCase()).lastIndexOf(tag);
                if (index >= 0) stack.length = Math.max(1, index);
                continue;
            }
            const element = tag === "html" ? documentElement : tag === "head" ? head : tag === "body" ? body : document.createElement(tag);
            if (!["html", "head", "body"].includes(tag)) stack[stack.length - 1].appendChild(element);
            const attributes = /([^\s=\/]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+)))?/g;
            for (const attr of token[2].matchAll(attributes)) {
                const name = attr[1].toLowerCase();
                const value = decode(attr[2] ?? attr[3] ?? attr[4] ?? "");
                if (element.setAttribute) element.setAttribute(name, value);
                else element[name === "class" ? "className" : name] = value;
                if (tag === "canvas" && ["width", "height"].includes(name)) element[name] = Number(value);
            }
            if (!voidTags.has(tag) && !token[0].endsWith("/>")) {
                if (["html", "head", "body"].includes(tag)) stack.length = 0;
                stack.push(element);
            }
        }
    };

    const storageData = (function () {
        try {
            return JSON.parse(native.storage.load()) || {};
        } catch (error) {
            return {};
        }
    })();

    function persistStorage() {
        try {
            native.storage.save(JSON.stringify(storageData));
        } catch (error) {
            reportError(error);
        }
    }

    const localStorage = new Proxy(storageData, {
        get(target, key) {
            if (key === "getItem") return (name) => (name in target ? target[name] : null);
            if (key === "setItem") {
                return (name, value) => {
                    target[name] = String(value);
                    persistStorage();
                };
            }
            if (key === "removeItem") {
                return (name) => {
                    delete target[name];
                    persistStorage();
                };
            }
            if (key === "clear") {
                return () => {
                    for (const name in target) delete target[name];
                    persistStorage();
                };
            }
            if (key === "key") return (index) => Object.keys(target)[index] ?? null;
            if (key === "length") return Object.keys(target).length;
            return target[key];
        },
        set(target, key, value) {
            target[key] = String(value);
            persistStorage();
            return true;
        },
        deleteProperty(target, key) {
            delete target[key];
            persistStorage();
            return true;
        },
    });

    const gamepadObjects = new Map();
    let gamepadRevision = -1;
    let gamepadSnapshotCache = null;
    let gamepadArrayRevision = -1;
    let gamepadArrayCache = null;

    class Gamepad {
        constructor(data) {
            this.hapticActuators = [];
            this.vibrationActuator = null;
            this.__update(data || {});
        }

        __update(data) {
            this.id = String(data.id || "");
            this.index = Number(data.index) || 0;
            this.connected = data.connected !== false;
            this.mapping = String(data.mapping || "standard");
            this.timestamp = Number(data.timestamp) || 0;
            this.axes = Array.from(data.axes || [], (value) => Number(value) || 0);
            this.buttons = Array.from(data.buttons || [], (button) => ({
                pressed: Boolean(button && button.pressed),
                touched: Boolean(button && button.touched),
                value: Number(button && button.value) || 0,
            }));
        }
    }

    function gamepadFromSnapshot(data) {
        const index = Number(data.index) || 0;
        let gamepad = gamepadObjects.get(index);
        if (!gamepad) {
            gamepad = new Gamepad(data);
            gamepadObjects.set(index, gamepad);
        } else {
            gamepad.__update(data);
        }
        return gamepad;
    }

    function gamepadRevisionNow() {
        if (!native.gamepad || typeof native.gamepad.revision !== "function") return null;
        try {
            return native.gamepad.revision();
        } catch (error) {
            reportError(error);
            return null;
        }
    }

    function nativeGamepads() {
        if (!native.gamepad || typeof native.gamepad.list !== "function") return [];
        const revision = gamepadRevisionNow();
        if (revision !== null && revision === gamepadRevision && gamepadSnapshotCache) {
            return gamepadSnapshotCache;
        }
        try {
            const list = JSON.parse(native.gamepad.list());
            const snapshots = Array.isArray(list) ? list : [];
            if (revision !== null) {
                gamepadRevision = revision;
                gamepadSnapshotCache = snapshots;
            }
            return snapshots;
        } catch (error) {
            reportError(error);
            return [];
        }
    }

    function gamepadsArray() {
        const revision = gamepadRevisionNow();
        if (revision !== null && revision === gamepadArrayRevision && gamepadArrayCache) {
            return gamepadArrayCache;
        }
        const gamepads = [];
        for (const data of nativeGamepads()) {
            const gamepad = gamepadFromSnapshot(data);
            gamepads[gamepad.index] = gamepad;
        }
        for (let index = 0; index < gamepads.length; index++) {
            if (!(index in gamepads)) gamepads[index] = null;
        }
        if (revision !== null) {
            gamepadArrayRevision = revision;
            gamepadArrayCache = gamepads;
        }
        return gamepads;
    }

    function disconnectGamepad(payload) {
        const index = Number(payload.index) || 0;
        const gamepad = gamepadObjects.get(index) || new Gamepad({
            id: payload.id,
            index,
            connected: false,
        });
        gamepad.connected = false;
        gamepadObjects.delete(index);
        gamepadRevision = -1;
        gamepadSnapshotCache = null;
        gamepadArrayRevision = -1;
        gamepadArrayCache = null;
        return gamepad;
    }

    const navigator = {
        userAgent: "kaplay-native",
        platform: native.env.platform,
        language: "en-US",
        getGamepads() {
            return gamepadsArray();
        },
        maxTouchPoints: 0,
        onLine: true,
        mediaCapabilities: {
            decodingInfo(config) {
                const type = (config && config.video && config.video.contentType) || "";
                return Promise.resolve({
                    supported: Boolean(native.video) && /^video\/mp4/i.test(type),
                    smooth: false,
                    powerEfficient: false,
                    keySystemAccess: null,
                });
            },
        },
        vibrate() {
            return false;
        },
    };

    function makeLocation(href) {
        let parsed;
        try {
            parsed = new URL(href || "kaplay-native://game/");
        } catch (error) {
            parsed = new URL("kaplay-native://game/");
        }
        return {
            href: parsed.href,
            origin: parsed.origin,
            protocol: parsed.protocol,
            host: parsed.host,
            hostname: parsed.hostname,
            pathname: parsed.pathname,
            search: parsed.search,
            hash: parsed.hash,
            username: parsed.username,
            password: parsed.password,
            port: parsed.port,
            get searchParams() {
                return new URLSearchParams(this.search);
            },
            toString() {
                return this.href;
            },
            assign(url) {
                const next = new URL(url, this.href);
                this.href = next.href;
                this.search = next.search;
                this.hash = next.hash;
                this.pathname = next.pathname;
            },
            replace(url) {
                this.assign(url);
            },
            reload() {},
        };
    }

    const location = makeLocation(native.env.startUrl);

    // -------------------------------------------------------------- networking

    function copyBytes(value) {
        const source = bytesView(value);
        const output = new Uint8Array(source.length);
        output.set(source);
        return output;
    }

    function blobBytes(blob) {
        const output = new Uint8Array(blob.size || 0);
        let offset = 0;
        for (const part of blob.__bytes || []) {
            output.set(part, offset);
            offset += part.length;
        }
        return output;
    }

    function requestBodyBytes(body) {
        if (body === undefined || body === null) return undefined;
        if (typeof body === "string") return utf8Encode(body);
        if (body instanceof URLSearchParams) return utf8Encode(body.toString());
        if (body instanceof Blob) return blobBytes(body);
        if (body instanceof ArrayBuffer || ArrayBuffer.isView(body)) return bytesView(body);
        return undefined;
    }

    class Headers {
        constructor(init) {
            this.__map = new Map();
            if (init instanceof Headers) {
                init.forEach((value, key) => this.set(key, value));
            } else if (Array.isArray(init)) {
                for (const entry of init) this.append(entry[0], entry[1]);
            } else if (init && typeof init === "object") {
                for (const key of Object.keys(init)) this.set(key, init[key]);
            }
        }

        append(name, value) {
            const key = String(name).toLowerCase();
            const existing = this.__map.get(key);
            this.__map.set(key, existing === undefined ? String(value) : existing + ", " + value);
        }

        set(name, value) {
            this.__map.set(String(name).toLowerCase(), String(value));
        }

        get(name) {
            const value = this.__map.get(String(name).toLowerCase());
            return value === undefined ? null : value;
        }

        has(name) {
            return this.__map.has(String(name).toLowerCase());
        }

        delete(name) {
            this.__map.delete(String(name).toLowerCase());
        }

        forEach(callback, thisArg) {
            for (const [key, value] of this.__map) callback.call(thisArg, value, key, this);
        }

        entries() {
            return this.__map.entries();
        }

        keys() {
            return this.__map.keys();
        }

        values() {
            return this.__map.values();
        }

        [Symbol.iterator]() {
            return this.__map.entries();
        }
    }

    const pendingFetches = new Map();
    let nextRequest = 1;

    class MessageEvent extends Event {
        constructor(type, init) {
            super(type, init);
            const options = init || {};
            this.data = options.data;
            this.origin = options.origin || "";
        }
    }

    class CloseEvent extends Event {
        constructor(type, init) {
            super(type, init);
            const options = init || {};
            this.code = Number(options.code || 0);
            this.reason = options.reason === undefined ? "" : String(options.reason);
            this.wasClean = Boolean(options.wasClean);
        }
    }

    const sockets = new Map();
    let nextSocket = 1;

    class WebSocket {
        constructor(url, protocols) {
            this.url = String(url);
            this.readyState = WebSocket.CONNECTING;
            this.binaryType = "blob";
            this.bufferedAmount = 0;
            this.extensions = "";
            this.protocol = "";
            this.onopen = null;
            this.onmessage = null;
            this.onerror = null;
            this.onclose = null;
            this.__id = nextSocket++;
            this.__listeners = makeListeners();
            if (Array.isArray(protocols)) {
                this.protocol = String(protocols[0] || "");
            } else if (protocols && typeof protocols === "object") {
                if (Array.isArray(protocols.protocols)) this.protocol = String(protocols.protocols[0] || "");
            } else if (protocols) {
                this.protocol = String(protocols);
            }
            if (!native.env.networkAllowed || !native.net) {
                setTimeoutFn(
                    () => this.__fail("network access is disabled, run the game with --allow-network"),
                    0,
                );
                return;
            }
            sockets.set(this.__id, this);
            let started = false;
            try {
                started = native.net.openSocket(this.__id, this.url);
            } catch (error) {
                setTimeoutFn(() => this.__fail(String((error && error.message) || error)), 0);
                return;
            }
            if (!started) {
                setTimeoutFn(() => this.__fail("could not start the websocket"), 0);
            }
        }

        addEventListener(type, fn) {
            this.__listeners.add(String(type), fn);
        }

        removeEventListener(type, fn) {
            this.__listeners.remove(String(type), fn);
        }

        dispatchEvent(event) {
            this.__emit(event);
            return true;
        }

        __emit(event) {
            event.target = this;
            event.currentTarget = this;
            this.__listeners.emit(event);
            const handler = this["on" + event.type];
            if (typeof handler === "function") {
                try {
                    handler.call(this, event);
                } catch (error) {
                    reportError(error);
                }
            }
        }

        send(data) {
            if (this.readyState !== WebSocket.OPEN) {
                throw new Error("WebSocket is not open");
            }
            if (typeof data === "string") {
                native.net.sendText(this.__id, data);
                return;
            }
            if (data instanceof Blob) {
                native.net.sendBinary(this.__id, blobBytes(data));
                return;
            }
            native.net.sendBinary(this.__id, bytesView(data));
        }

        close(code, reason) {
            if (this.readyState >= WebSocket.CLOSING) return;
            this.readyState = WebSocket.CLOSING;
            native.net.closeSocket(
                this.__id,
                code === undefined ? 1000 : Number(code),
                reason === undefined ? "" : String(reason),
            );
        }

        __handleOpen() {
            if (this.readyState !== WebSocket.CONNECTING) return;
            this.readyState = WebSocket.OPEN;
            this.__emit(new Event("open"));
        }

        __handleMessage(text, bytes) {
            let data;
            if (text) data = utf8Decode(bytes);
            else if (this.binaryType === "arraybuffer") data = copyBytes(bytes).buffer;
            else data = new Blob([bytes]);
            this.__emit(new MessageEvent("message", { data, origin: "" }));
        }

        __handleClose(code, reason) {
            if (this.readyState === WebSocket.CLOSED) return;
            this.readyState = WebSocket.CLOSED;
            sockets.delete(this.__id);
            this.__emit(new CloseEvent("close", {
                code,
                reason,
                wasClean: code === 1000 || code === 1005,
            }));
        }

        __handleError(message) {
            const event = new Event("error");
            event.message = message;
            this.__emit(event);
        }

        __fail(message) {
            if (this.readyState !== WebSocket.CONNECTING) return;
            this.__handleError(message);
            this.__handleClose(1006, "");
        }
    }

    WebSocket.CONNECTING = 0;
    WebSocket.OPEN = 1;
    WebSocket.CLOSING = 2;
    WebSocket.CLOSED = 3;

    global.__kapNative_onSocketOpen = function (id) {
        const socket = sockets.get(id);
        if (socket) socket.__handleOpen();
    };

    global.__kapNative_onSocketMessage = function (id, length, text) {
        const socket = sockets.get(id);
        if (!socket) return;
        const bytes = new Uint8Array(length);
        const copied = native.net.readMessage(id, bytes);
        if (copied < 0) return;
        socket.__handleMessage(Boolean(text), bytes);
    };

    global.__kapNative_onSocketClose = function (id, code, reason) {
        const socket = sockets.get(id);
        if (socket) socket.__handleClose(Number(code), String(reason));
    };

    global.__kapNative_onSocketError = function (id, message) {
        const socket = sockets.get(id);
        if (socket) socket.__handleError(String(message));
    };

    class AbortSignal {
        constructor() {
            this.aborted = false;
            this.reason = undefined;
            this.onabort = null;
            this.__listeners = makeListeners();
        }

        addEventListener(type, fn) {
            this.__listeners.add(String(type), fn);
        }

        removeEventListener(type, fn) {
            this.__listeners.remove(String(type), fn);
        }

        throwIfAborted() {
            if (this.aborted) throw this.reason;
        }

        static abort(reason) {
            const signal = new AbortSignal();
            signal.aborted = true;
            signal.reason = reason === undefined ? new Error("This operation was aborted") : reason;
            return signal;
        }

        static timeout(milliseconds) {
            const signal = new AbortSignal();
            signal.reason = new Error("The operation timed out");
            signal.reason.name = "TimeoutError";
            setTimeoutFn(() => {
                if (!signal.aborted) signal.aborted = true;
            }, Number(milliseconds));
            return signal;
        }
    }

    class AbortController {
        constructor() {
            this.signal = new AbortSignal();
        }

        abort(reason) {
            if (this.signal.aborted) return;
            this.signal.aborted = true;
            this.signal.reason = reason === undefined
                ? new Error("This operation was aborted")
                : reason;
            const event = new Event("abort");
            event.target = this.signal;
            this.signal.__listeners.emit(event);
            if (typeof this.signal.onabort === "function") this.signal.onabort(event);
        }
    }

    function makeResponse(bytes, url, init) {
        const options = init || {};
        const status = options.status === undefined ? 200 : Number(options.status);
        const headers = options.headers instanceof Headers
            ? options.headers
            : new Headers(options.headers || {});
        const body = bytesView(bytes);
        const statusText = options.statusText === undefined
            ? (status === 200 ? "OK" : "")
            : String(options.statusText);
        return {
            ok: status >= 200 && status < 300,
            status,
            statusText,
            url,
            redirected: false,
            type: "basic",
            headers,
            json() {
                try {
                    return Promise.resolve(JSON.parse(utf8Decode(body)));
                } catch (error) {
                    return Promise.reject(error);
                }
            },
            text() {
                return Promise.resolve(utf8Decode(body));
            },
            arrayBuffer() {
                return Promise.resolve(copyBytes(body).buffer);
            },
            blob() {
                return Promise.resolve(
                    new Blob([body], { type: headers.get("content-type") || "" }),
                );
            },
        };
    }

    function resolveFetchURL(url) {
        if (/^https?:/i.test(url)) return url;
        if (/^[a-z][a-z0-9+.-]*:/i.test(url)) return null;
        if (!/^https?:/i.test(location.protocol)) return null;
        try {
            return new URL(url, location.href).href;
        } catch (error) {
            return null;
        }
    }

    function fetchResource(input, init) {
        const options = init || {};
        let url;
        let method = options.method || "GET";
        let headers = options.headers;
        let body = options.body;
        if (input && typeof input === "object") {
            url = String(input.url === undefined ? input : input.url);
            if (!options.method && input.method) method = input.method;
            if (headers === undefined && input.headers) headers = input.headers;
            if (body === undefined && input.body !== undefined) body = input.body;
        } else {
            url = String(input);
        }
        const requestHeaders = headers instanceof Headers ? headers : new Headers(headers || {});
        const signal = options.signal;
        return new Promise((resolve, reject) => {
            const rejectFetch = () =>
                setTimeoutFn(() => reject(new TypeError("Failed to fetch " + url)), 0);
            if (signal && signal.aborted) {
                reject(signal.reason || new Error("The operation was aborted"));
                return;
            }
            const data = decodeDataURL(url);
            if (data) {
                resolve(makeResponse(data, url));
                return;
            }
            if (/^blob:/i.test(url)) {
                const stored = objectURLs.get(url);
                if (stored) {
                    resolve(makeResponse(blobBytes(stored), url, {
                        headers: { "content-type": stored.type || "" },
                    }));
                } else {
                    rejectFetch();
                }
                return;
            }
            const relative = !/^[a-z][a-z0-9+.-]*:/i.test(url);
            if (relative) {
                const size = native.fs.size(url);
                if (size >= 0) {
                    const bytes = new Uint8Array(size);
                    native.fs.read(url, bytes);
                    setTimeoutFn(() => resolve(makeResponse(bytes, url)), 0);
                    return;
                }
            }
            const target = resolveFetchURL(url);
            if (!target) {
                rejectFetch();
                return;
            }
            if (!native.env.networkAllowed || !native.net) {
                reject(
                    new TypeError(
                        "Failed to fetch " + target + ": network access is disabled, run the game with --allow-network",
                    ),
                );
                return;
            }
            const id = nextRequest++;
            pendingFetches.set(id, { resolve, reject, url: target, signal });
            if (signal && typeof signal.addEventListener === "function") {
                signal.addEventListener("abort", () => {
                    const pending = pendingFetches.get(id);
                    if (!pending) return;
                    pendingFetches.delete(id);
                    pending.reject(signal.reason || new Error("The operation was aborted"));
                });
            }
            let started = false;
            try {
                const pairs = [];
                requestHeaders.forEach((value, key) => pairs.push([key, value]));
                started = native.net.fetch(
                    id,
                    String(method).toUpperCase(),
                    target,
                    JSON.stringify(pairs),
                    requestBodyBytes(body),
                );
            } catch (error) {
                pendingFetches.delete(id);
                reject(error);
                return;
            }
            if (!started) {
                pendingFetches.delete(id);
                reject(new TypeError("Failed to fetch " + target));
            }
        });
    }

    global.__kapNative_onFetch = function (id, url, status, statusText, headersJson, length, ok) {
        const pending = pendingFetches.get(id);
        if (!pending) return;
        pendingFetches.delete(id);
        let headers = new Headers();
        try {
            headers = new Headers(JSON.parse(headersJson));
        } catch (error) {
            reportError(error);
        }
        const bytes = new Uint8Array(length);
        const copied = native.net.readResponse(id, bytes);
        if (copied < 0) {
            pending.reject(new TypeError("Failed to fetch " + url));
            return;
        }
        pending.resolve(makeResponse(bytes, url, { status, statusText, headers }));
    };

    global.__kapNative_onFetchError = function (id, message) {
        const pending = pendingFetches.get(id);
        if (!pending) return;
        pendingFetches.delete(id);
        const error = new TypeError("Failed to fetch " + pending.url + ": " + message);
        error.cause = new Error(message);
        pending.reject(error);
    };

    const console = {
        log(...args) {
            native.env.log("info", args.map(String).join(" "));
        },
        info(...args) {
            native.env.log("info", args.map(String).join(" "));
        },
        debug(...args) {
            native.env.log("debug", args.map(String).join(" "));
        },
        warn(...args) {
            native.env.log("warn", args.map(String).join(" "));
        },
        error(...args) {
            native.env.log("error", args.map(String).join(" "));
        },
        trace(...args) {
            native.env.log("debug", args.map(String).join(" "));
        },
    };

    // ----------------------------------------------------------- game platform

    const steam = {
        available: () => native.steam.available(),
        playerName: () => native.steam.playerName(),
        steamId: () => native.steam.steamId(),
        unlockAchievement: (name) => native.steam.unlockAchievement(String(name)),
        isAchievementUnlocked: (name) => native.steam.isAchievementUnlocked(String(name)),
        storeStats: () => native.steam.storeStats(),
        setStatInt: (name, value) => native.steam.setStatInt(String(name), Number(value) | 0),
        getStatInt: (name) => native.steam.getStatInt(String(name)),
        setStatFloat: (name, value) => native.steam.setStatFloat(String(name), Number(value)),
        getStatFloat: (name) => native.steam.getStatFloat(String(name)),
        writeCloud: (name, text) => native.steam.writeCloud(String(name), String(text)),
        readCloud: (name) => native.steam.readCloud(String(name)),
        cloudExists: (name) => native.steam.cloudExists(String(name)),
        deleteCloud: (name) => native.steam.deleteCloud(String(name)),
        runCallbacks: () => native.steam.runCallbacks(),
    };

    const updates = native.updates
        ? {
              status: () => JSON.parse(native.updates.snapshot()),
              apply: () => native.updates.apply(),
          }
        : {
              status: () => ({ status: "idle", required: false, progress: 0 }),
              apply: () => false,
          };

    const kaplayNative = {
        version: native.env.hostVersion,
        platform: native.env.platform,
        steam,
        updates,
    };

    // ------------------------------------------------- system browser hand-off

    let authWindow = null;

    function openBrowserWindow(url) {
        if (authWindow || typeof url !== "string" || !/^https?:\/\//i.test(url)) {
            return null;
        }
        if (!native.auth || typeof native.auth.open !== "function") {
            return null;
        }
        if (!native.auth.open(url)) {
            return null;
        }
        authWindow = {
            closed: false,
            close() {
                this.closed = true;
            },
            focus() {},
            blur() {},
            postMessage() {},
        };
        return authWindow;
    }

    global.__kapNative_onAuthCallback = function (payloadJson) {
        const pending = authWindow;
        authWindow = null;
        if (pending) pending.closed = true;
        if (!payloadJson) return;
        let data;
        try {
            data = JSON.parse(payloadJson);
        } catch (error) {
            console.error("auth callback:", error);
            return;
        }
        const event = makeEvent("message", {});
        event.data = data;
        windowListeners.emit(event);
    };

    // ------------------------------------------------------------------ export

    const define = (name, value) => {
        try {
            Object.defineProperty(global, name, { value, writable: true, configurable: true });
            return;
        } catch {}
        try {
            global[name] = value;
        } catch {}
    };
    define("window", global);
    define("kaplayNative", kaplayNative);
    define("self", global);
    define("document", document);
    define("navigator", navigator);
    define("location", location);
    define("screen", screen);
    define("open", openBrowserWindow);
    define("console", console);
    define("localStorage", localStorage);
    define("Image", Image);
    define("ImageData", ImageData);
    define("Gamepad", Gamepad);
    define("Blob", Blob);
    define("Headers", Headers);
    define("WebSocket", WebSocket);
    define("MessageEvent", MessageEvent);
    define("CloseEvent", CloseEvent);
    define("AbortController", AbortController);
    define("AbortSignal", AbortSignal);
    define("URL", URL);
    define("crypto", crypto);
    define("URLSearchParams", URLSearchParams);
    global.matchMedia = (query) => ({
        matches: false,
        media: String(query),
        onchange: null,
        addEventListener() {},
        removeEventListener() {},
        addListener() {},
        removeListener() {},
        dispatchEvent() {
            return false;
        },
    });
    define("FontFace", FontFace);
    global.Audio = HTMLAudioElement;
    global.HTMLVideoElement = HTMLVideoElement;
    define("AudioBuffer", AudioBuffer);
    define("AudioContext", AudioContext);
    define("AudioNode", AudioNode);
    define("AudioParam", AudioParam);
    define("GainNode", GainNode);
    define("StereoPannerNode", StereoPannerNode);
    define("AudioBufferSourceNode", AudioBufferSourceNode);
    define("OscillatorNode", OscillatorNode);
    define("BiquadFilterNode", BiquadFilterNode);
    define("DelayNode", DelayNode);
    define("WaveShaperNode", WaveShaperNode);
    define("DynamicsCompressorNode", DynamicsCompressorNode);
    define("ConvolverNode", ConvolverNode);
    define("ChannelSplitterNode", ChannelSplitterNode);
    define("ChannelMergerNode", ChannelMergerNode);
    define("IIRFilterNode", IIRFilterNode);
    define("PannerNode", PannerNode);
    define("ConstantSourceNode", ConstantSourceNode);
    define("AnalyserNode", AnalyserNode);
    define("PeriodicWave", PeriodicWave);
    define("MediaElementAudioSourceNode", MediaElementAudioSourceNode);
    global.webkitAudioContext = AudioContext;
    define("ResizeObserver", ResizeObserver);
    define("MutationObserver", MutationObserver);
    define("HTMLCanvasElement", HTMLCanvasElement);
    define("HTMLElement", HTMLElement);
    define("HTMLInputElement", HTMLInputElement);
    define("HTMLTextAreaElement", HTMLTextAreaElement);
    define("HTMLFormElement", HTMLFormElement);
    define("Event", Event);
    define("MouseEvent", MouseEvent);
    define("KeyboardEvent", KeyboardEvent);
    define("WheelEvent", WheelEvent);
    define("CustomEvent", CustomEvent);
    define("TextEncoder", TextEncoder);
    define("TextDecoder", TextDecoder);
    global.fetch = fetchResource;
    global.close = () => native.env.quit();
    global.atob = atob;
    global.btoa = btoa;
    global.requestAnimationFrame = requestAnimationFrame;
    global.cancelAnimationFrame = cancelAnimationFrame;
    global.addEventListener = (type, fn) => windowListeners.add(type, fn);
    global.removeEventListener = (type, fn) => windowListeners.remove(type, fn);
    global.dispatchEvent = (event) => {
        windowListeners.emit(event);
        return true;
    };
    global.setTimeout = setTimeoutFn;
    global.clearTimeout = clearTimer;
    global.setInterval = setIntervalFn;
    global.clearInterval = clearTimer;
    global.performance = { now: () => timerNow() };
    global.kaplayjs_assetsAliases = {};
    global.onerror = null;

    Object.defineProperty(global, "innerWidth", {
        get: () => screen.width,
        configurable: true,
    });
    Object.defineProperty(global, "innerHeight", {
        get: () => screen.height,
        configurable: true,
    });
    Object.defineProperty(global, "devicePixelRatio", {
        get: () => screen.dpr,
        configurable: true,
    });

    if (typeof Promise.withResolvers !== "function") {
        Promise.withResolvers = function () {
            let resolve;
            let reject;
            const promise = new Promise((res, rej) => {
                resolve = res;
                reject = rej;
            });
            return { promise, resolve, reject };
        };
    }
})();
