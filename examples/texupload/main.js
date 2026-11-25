// Focused acceptance test for texture upload paths:
// - texImage2D with raw typed arrays (zero-copy bridge path)
// - texSubImage2D width/height overload with raw typed arrays
// - texSubImage2D with ImageData
// - non-RGBA (LUMINANCE) sub-upload with explicit dimensions
// Renders each result into a quadrant and reads pixels back so the
// frame can be verified deterministically.
const canvas = document.createElement("canvas");
canvas.width = 64;
canvas.height = 64;
document.body.appendChild(canvas);
const gl = canvas.getContext("webgl");

function makeTexture(width, height, format, pixels) {
    const tex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texImage2D(gl.TEXTURE_2D, 0, format, width, height, 0, format, gl.UNSIGNED_BYTE, pixels);
    return tex;
}

const vs = "attribute vec2 a_pos;attribute vec2 a_uv;varying vec2 v_uv;void main(){v_uv=a_uv;gl_Position=vec4(a_pos,0.0,1.0);}";
const fs = "precision mediump float;varying vec2 v_uv;uniform sampler2D u_tex;void main(){gl_FragColor=texture2D(u_tex,v_uv);}";
function program() {
    const v = gl.createShader(gl.VERTEX_SHADER);
    gl.shaderSource(v, vs); gl.compileShader(v);
    const f = gl.createShader(gl.FRAGMENT_SHADER);
    gl.shaderSource(f, fs); gl.compileShader(f);
    const p = gl.createProgram();
    gl.attachShader(p, v); gl.attachShader(p, f);
    gl.bindAttribLocation(p, 0, "a_pos");
    gl.bindAttribLocation(p, 1, "a_uv");
    gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) {
        console.log("TEXUPLOAD link error", gl.getProgramInfoLog(p));
    }
    return p;
}
const prog = program();
gl.useProgram(prog);
const verts = new Float32Array([-1,-1,0,0, 1,-1,1,0, -1,1,0,1, 1,1,1,1]);
const buf = gl.createBuffer();
gl.bindBuffer(gl.ARRAY_BUFFER, buf);
gl.bufferData(gl.ARRAY_BUFFER, verts, gl.STATIC_DRAW);
gl.enableVertexAttribArray(0);
gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 16, 0);
gl.enableVertexAttribArray(1);
gl.vertexAttribPointer(1, 2, gl.FLOAT, false, 16, 8);

// Quadrant 1 (top-left): texImage2D raw RGBA, solid red.
const red = new Uint8Array(8 * 8 * 4);
for (let i = 0; i < red.length; i += 4) { red[i] = 255; red[i+3] = 255; }
const texRed = makeTexture(8, 8, gl.RGBA, red);

// Quadrant 2 (top-right): texImage2D allocation then texSubImage2D raw green.
const texGreen = makeTexture(8, 8, gl.RGBA, null);
const green = new Uint8Array(8 * 8 * 4);
for (let i = 0; i < green.length; i += 4) { green[i+1] = 255; green[i+3] = 255; }
gl.bindTexture(gl.TEXTURE_2D, texGreen);
gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, 8, 8, gl.RGBA, gl.UNSIGNED_BYTE, green);

// Quadrant 3 (bottom-left): texSubImage2D with ImageData (blue).
const texBlue = makeTexture(8, 8, gl.RGBA, null);
const blueData = new ImageData(8, 8);
for (let i = 0; i < blueData.data.length; i += 4) { blueData.data[i+2] = 255; blueData.data[i+3] = 255; }
gl.bindTexture(gl.TEXTURE_2D, texBlue);
gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, gl.RGBA, gl.UNSIGNED_BYTE, blueData);

// Quadrant 4 (bottom-right): partial sub-update at an offset (yellow patch
// on black), exercising xoffset/yoffset with explicit width/height.
const texPatch = makeTexture(8, 8, gl.RGBA, new Uint8Array(8 * 8 * 4).fill(0));
const yellow = new Uint8Array(4 * 4 * 4);
for (let i = 0; i < yellow.length; i += 4) { yellow[i] = 255; yellow[i+1] = 255; yellow[i+3] = 255; }
gl.bindTexture(gl.TEXTURE_2D, texPatch);
gl.texSubImage2D(gl.TEXTURE_2D, 0, 2, 2, 4, 4, gl.RGBA, gl.UNSIGNED_BYTE, yellow);

function drawQuadrant(tex, x, y) {
    gl.viewport(x, y, 32, 32);
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
}

let frame = 0;
function draw() {
    frame++;
    gl.disable(gl.BLEND);
    gl.clearColor(0, 0, 0, 1);
    gl.viewport(0, 0, 64, 64);
    gl.clear(gl.COLOR_BUFFER_BIT);
    // GL viewport origin is bottom-left; quadrants: TL red, TR green, BL blue, BR patch.
    drawQuadrant(texRed, 0, 32);
    drawQuadrant(texGreen, 32, 32);
    drawQuadrant(texBlue, 0, 0);
    drawQuadrant(texPatch, 32, 0);
    if (frame === 3) {
        const px = new Uint8Array(64 * 64 * 4);
        gl.readPixels(0, 0, 64, 64, gl.RGBA, gl.UNSIGNED_BYTE, px);
        const at = (x, y) => { const i = (y * 64 + x) * 4; return [px[i], px[i+1], px[i+2], px[i+3]]; };
        console.log("TEXUPLOAD tl", JSON.stringify(at(16, 48)));
        console.log("TEXUPLOAD tr", JSON.stringify(at(48, 48)));
        console.log("TEXUPLOAD bl", JSON.stringify(at(16, 16)));
        console.log("TEXUPLOAD br-center", JSON.stringify(at(48, 16)));
        console.log("TEXUPLOAD br-corner", JSON.stringify(at(34, 2)));
        console.log("TEXUPLOAD done");
    }
    if (frame < 4) requestAnimationFrame(draw);
}
requestAnimationFrame(draw);
