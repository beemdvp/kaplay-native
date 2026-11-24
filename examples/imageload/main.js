// Focused acceptance test for async image decode: loads a generated PNG
// data URL through Image.src, verifies onload dimensions and pixel content
// by uploading the decoded image to a GL texture and reading it back, and
// checks that a src switch after load still completes.
const canvas = document.createElement("canvas");
canvas.width = 32;
canvas.height = 32;
document.body.appendChild(canvas);
const gl = canvas.getContext("webgl");

// Build a 16x8 solid-green PNG via a raw RGBA texImage2D + readPixels is not
// possible without shaders; instead embed a tiny known PNG data URL.
// 2x2 green PNG (generated once, verified below by readback).
const GREEN_PNG = "data:image/png;base64," +
    "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAADklEQVR4nGNg+A+FMAYAQ84H" +
    "+fei4u8AAAAASUVORK5CYII=";

const vs = "attribute vec2 a_pos;attribute vec2 a_uv;varying vec2 v_uv;void main(){v_uv=a_uv;gl_Position=vec4(a_pos,0.0,1.0);}";
const fs = "precision mediump float;varying vec2 v_uv;uniform sampler2D u_tex;void main(){gl_FragColor=texture2D(u_tex,v_uv);}";
const v = gl.createShader(gl.VERTEX_SHADER);
gl.shaderSource(v, vs); gl.compileShader(v);
const f = gl.createShader(gl.FRAGMENT_SHADER);
gl.shaderSource(f, fs); gl.compileShader(f);
const prog = gl.createProgram();
gl.attachShader(prog, v); gl.attachShader(prog, f);
gl.bindAttribLocation(prog, 0, "a_pos");
gl.bindAttribLocation(prog, 1, "a_uv");
gl.linkProgram(prog);
gl.useProgram(prog);
const buf = gl.createBuffer();
gl.bindBuffer(gl.ARRAY_BUFFER, buf);
gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1,-1,0,0, 1,-1,1,0, -1,1,0,1, 1,1,1,1]), gl.STATIC_DRAW);
gl.enableVertexAttribArray(0);
gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 16, 0);
gl.enableVertexAttribArray(1);
gl.vertexAttribPointer(1, 2, gl.FLOAT, false, 16, 8);

let loaded = 0;
let failed = 0;
const image = new Image();
image.onload = () => {
    loaded++;
    console.log("IMAGELOAD dims", image.width, image.height, image.naturalWidth, image.naturalHeight);
    const tex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, image);
    gl.viewport(0, 0, 32, 32);
    gl.clearColor(0, 0, 0, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    const px = new Uint8Array(32 * 32 * 4);
    gl.readPixels(0, 0, 32, 32, gl.RGBA, gl.UNSIGNED_BYTE, px);
    const i = (16 * 32 + 16) * 4;
    console.log("IMAGELOAD pixel", px[i], px[i+1], px[i+2], px[i+3]);
    image.onload = () => {
        loaded++;
        console.log("IMAGELOAD second", image.width, image.height);
        console.log("IMAGELOAD done", loaded, failed);
    };
    image.src = GREEN_PNG;
};
image.onerror = () => {
    failed++;
    console.log("IMAGELOAD error");
};
image.src = GREEN_PNG;

let frame = 0;
function tick() {
    frame++;
    if (frame < 60 && loaded < 2) requestAnimationFrame(tick);
}
requestAnimationFrame(tick);
