(function () {
    "use strict";

    const native = __kapNative;
    const results = {};
    const record = (name, value) => {
        results[name] = value === undefined ? "undefined" : value;
    };
    const checksum = (bytes) => {
        let total = 0;
        for (let index = 0; index < bytes.length; index++) {
            total = (total * 31 + bytes[index]) >>> 0;
        }
        return total;
    };
    const encoder = new TextEncoder();

    record("env.hostVersion", native.env.hostVersion);
    record("env.gameName", native.env.gameName);
    record("env.networkAllowed", native.env.networkAllowed);
    record("env.startUrl", native.env.startUrl);
    record("env.now", Number.isFinite(native.env.now()));
    record("window.size", [window.innerWidth, window.innerHeight]);
    record("window.devicePixelRatio", window.devicePixelRatio);
    native.window.setTitle("bridge");
    document.title = "bridge";
    record("window.title", document.title);
    native.window.setFullscreen(true);
    record("window.fullscreen", native.window.isFullscreen());
    native.window.setFullscreen(false);
    record("window.fullscreenOff", native.window.isFullscreen());
    native.window.setCursorLocked(true);
    record("window.cursorLocked", native.window.isCursorLocked());
    native.window.setCursorLocked(false);
    native.window.setCursor("pointer");
    record("window.cursorOff", native.window.isCursorLocked());

    record("fs.exists", native.fs.exists("asset.txt"));
    record("fs.size", native.fs.size("asset.txt"));
    const asset = new Uint8Array(native.fs.size("asset.txt"));
    record("fs.read", native.fs.read("asset.txt", asset));
    record("fs.checksum", checksum(asset));
    record("fs.missingExists", native.fs.exists("missing.txt"));
    record("fs.missingSize", native.fs.size("missing.txt"));
    record("fs.missingRead", native.fs.read("missing.txt", new Uint8Array(4)));

    localStorage.clear();
    localStorage.setItem("bridge", "value");
    record("storage.item", localStorage.getItem("bridge"));
    localStorage.removeItem("bridge");
    record("storage.cleared", localStorage.getItem("bridge") === null);
    native.storage.save('{"native":true}');
    record("storage.raw", native.storage.load());

    record("font.before", native.font.has("bridge-font"));
    native.font.register("bridge-font", encoder.encode("not-a-real-font"));
    record("font.after", native.font.has("bridge-font"));

    const sprite = document.createElement("canvas");
    sprite.width = 8;
    sprite.height = 8;
    const spriteContext = sprite.getContext("2d");
    spriteContext.fillStyle = "#ff8800";
    spriteContext.font = "10px monospace";
    spriteContext.textBaseline = "top";
    spriteContext.fillText("x", 0, 0);
    const spriteUrl = sprite.toDataURL("image/png");
    record("image.pngHeader", spriteUrl.slice(0, 22));
    record("image.pngLength", spriteUrl.length);

    const image = new Image();
    image.src = spriteUrl;
    record("image.size", [image.width, image.height]);

    const composed = document.createElement("canvas");
    composed.width = 8;
    composed.height = 8;
    const composedContext = composed.getContext("2d");
    composedContext.drawImage(image, 0, 0);
    record("image.drawChecksum", checksum(composedContext.getImageData(0, 0, 8, 8).data));

    const canvas = document.createElement("canvas");
    canvas.width = 32;
    canvas.height = 16;
    const context = canvas.getContext("2d");
    context.fillStyle = "#3366cc";
    context.strokeStyle = "#000000";
    context.lineWidth = 1;
    context.lineJoin = "round";
    context.font = "10px monospace";
    context.textAlign = "left";
    context.textBaseline = "top";
    context.fillText("hi", 1, 1);
    context.strokeText("hi", 1, 1);
    context.clearRect(0, 12, 32, 4);
    const metrics = context.measureText("hi");
    record("canvas.measureText", [
        Math.round(metrics.width),
        Math.round(metrics.actualBoundingBoxAscent),
        Math.round(metrics.actualBoundingBoxDescent),
    ]);
    const pixels = context.getImageData(0, 0, 32, 16);
    record("canvas.checksum", checksum(pixels.data));

    const target = document.createElement("canvas");
    target.width = 32;
    target.height = 16;
    const targetContext = target.getContext("2d");
    targetContext.putImageData(pixels, 0, 0);
    targetContext.drawImage(canvas, 0, 0);
    record("canvas.drawChecksum", checksum(targetContext.getImageData(0, 0, 32, 16).data));
    record("canvas.pngLength", target.toDataURL("image/png").length);

    const glCanvas = document.createElement("canvas");
    glCanvas.width = 16;
    glCanvas.height = 16;
    document.body.appendChild(glCanvas);
    const gl = glCanvas.getContext("webgl");
    record("gl.drawingBuffer", [gl.drawingBufferWidth, gl.drawingBufferHeight]);
    gl.viewport(0, 0, 16, 16);
    gl.clearColor(0.25, 0.5, 0.75, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    const cleared = new Uint8Array(16 * 16 * 4);
    gl.readPixels(0, 0, 16, 16, gl.RGBA, gl.UNSIGNED_BYTE, cleared);
    record("gl.clearedPixel", [cleared[0], cleared[1], cleared[2], cleared[3]]);

    const vertex = "attribute vec2 a_pos;uniform mat4 u_matrix;void main(){gl_Position=u_matrix*vec4(a_pos,0.0,1.0);}";
    const fragment = "precision mediump float;void main(){gl_FragColor=vec4(1.0,0.5,0.25,1.0);}";
    const vertexShader = gl.createShader(gl.VERTEX_SHADER);
    gl.shaderSource(vertexShader, vertex);
    gl.compileShader(vertexShader);
    const fragmentShader = gl.createShader(gl.FRAGMENT_SHADER);
    gl.shaderSource(fragmentShader, fragment);
    gl.compileShader(fragmentShader);
    const program = gl.createProgram();
    gl.attachShader(program, vertexShader);
    gl.attachShader(program, fragmentShader);
    gl.linkProgram(program);
    record("gl.linkStatus", gl.getProgramParameter(program, gl.LINK_STATUS));
    record("gl.attribLocation", gl.getAttribLocation(program, "a_pos"));
    gl.useProgram(program);
    const matrix = new Float32Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
    gl.uniformMatrix4fv(gl.getUniformLocation(program, "u_matrix"), false, matrix);

    const vertices = new Float32Array([-1, -1, 1, -1, -1, 1]);
    const vertexBuffer = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, vertexBuffer);
    gl.bufferData(gl.ARRAY_BUFFER, vertices, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 8, 0);
    const indices = new Uint16Array([0, 1, 2]);
    const elementBuffer = gl.createBuffer();
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, elementBuffer);
    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, indices, gl.STATIC_DRAW);
    gl.drawElements(gl.TRIANGLES, 3, gl.UNSIGNED_SHORT, 0);

    const framebuffer = gl.createFramebuffer();
    const renderbuffer = gl.createRenderbuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
    gl.bindRenderbuffer(gl.RENDERBUFFER, renderbuffer);
    gl.renderbufferStorage(gl.RENDERBUFFER, gl.RGBA4, 16, 16);
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.RENDERBUFFER, renderbuffer);
    record("gl.framebufferComplete", gl.checkFramebufferStatus(gl.FRAMEBUFFER) === gl.FRAMEBUFFER_COMPLETE);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.deleteFramebuffer(framebuffer);
    gl.deleteRenderbuffer(renderbuffer);
    gl.deleteBuffer(vertexBuffer);
    gl.deleteBuffer(elementBuffer);
    gl.deleteShader(vertexShader);
    gl.deleteShader(fragmentShader);
    gl.deleteProgram(program);

    record("gl.screenshotLength", glCanvas.toDataURL("image/png").length);

    record("net.fetch", native.net.fetch(1, "GET", "https://example.invalid/", "", new Uint8Array(0)));
    record("net.openSocket", native.net.openSocket(2, "ws://example.invalid/"));
    record("net.readResponse", native.net.readResponse(1, new Uint8Array(8)));
    record("net.readMessage", native.net.readMessage(2, new Uint8Array(8)));
    native.net.sendText(2, "hello");
    native.net.sendBinary(2, encoder.encode("hello"));
    native.net.closeSocket(2, 1000, "done");

    record("audio.available", native.audio.available());
    record("audio.sampleRate", native.audio.sampleRate());
    record("audio.deviceTime", native.audio.currentTime() >= 0);
    record("audio.bufferInfo", native.audio.bufferInfo(4096, new Float32Array(3)));
    record("audio.upload", native.audio.uploadBuffer(48000, 1, new Float32Array(64)));
    record("audio.position", native.audio.position(4096));
    record("audio.duration", native.audio.duration(4096));
    native.audio.setPaused(true);
    native.audio.setPaused(false);
    native.audio.freeBuffer(4096);

    record("steam.available", native.steam.available());
    record("steam.playerName", native.steam.playerName());
    record("steam.steamId", native.steam.steamId());
    record("steam.unlock", native.steam.unlockAchievement("BRIDGE"));
    record("steam.achievement", native.steam.isAchievementUnlocked("BRIDGE"));
    record("steam.storeStats", native.steam.storeStats());
    record("updates.snapshot", JSON.parse(native.updates.snapshot()));
    record("updates.apply", native.updates.apply());
    record("steam.setStatInt", native.steam.setStatInt("bridge", 1));
    record("steam.getStatInt", native.steam.getStatInt("bridge"));
    record("steam.setStatFloat", native.steam.setStatFloat("bridge", 1.5));
    record("steam.getStatFloat", native.steam.getStatFloat("bridge"));
    record("steam.writeCloud", native.steam.writeCloud("bridge.txt", "value"));
    record("steam.readCloud", native.steam.readCloud("bridge.txt"));
    record("steam.cloudExists", native.steam.cloudExists("bridge.txt"));
    record("steam.deleteCloud", native.steam.deleteCloud("bridge.txt"));
    native.steam.runCallbacks();

    native.env.log("info", "BRIDGE " + JSON.stringify(results));
})();
