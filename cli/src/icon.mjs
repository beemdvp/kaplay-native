import { Data, NtExecutable, NtExecutableResource, Resource } from "resedit";
import { PNG } from "pngjs";

const MIN_SIZE = 256;
const MAX_SIZE = 2048;
// 256x256 crosses the 16-bit X11 property request length; winit then writes an empty icon.
const WINDOW_SIZE = 128;
const ICO_SIZES = [16, 32, 48, 64, 128, 256];
const ICNS_TYPES = [
    ["icp4", 16],
    ["icp5", 32],
    ["icp6", 64],
    ["ic07", 128],
    ["ic08", 256],
    ["ic09", 512],
    ["ic10", 1024],
    ["ic11", 32],
    ["ic12", 64],
    ["ic13", 256],
    ["ic14", 512],
];

export function isMachO(bytes) {
    if (bytes.length < 4) return false;
    const magic = bytes.readUInt32BE(0);
    return magic === 0xfeedface || magic === 0xfeedfacf || magic === 0xcefaedfe || magic === 0xcffaedfe
        || magic === 0xcafebabe || magic === 0xbebafeca;
}

export function isPe(bytes) {
    return bytes.length >= 2 && bytes[0] === 0x4d && bytes[1] === 0x5a;
}

export function prepareIcon(bytes) {
    let image;
    try {
        image = PNG.sync.read(bytes);
    } catch (error) {
        throw new Error(`not a readable PNG: ${error.message}`);
    }
    if (image.width !== image.height) {
        throw new Error(`icon must be square, got ${image.width}x${image.height}`);
    }
    if (image.width < MIN_SIZE || image.width > MAX_SIZE) {
        throw new Error(`icon must be ${MIN_SIZE} to ${MAX_SIZE} pixels, got ${image.width}`);
    }
    return {
        windowPng: encode(resize(image, WINDOW_SIZE)),
        ico: buildIco(image),
        icns: buildIcns(image),
    };
}

function resize(source, size) {
    if (source.width === size) return source;
    const scale = source.width / size;
    const data = Buffer.alloc(size * size * 4);
    for (let y = 0; y < size; y++) {
        const top = Math.floor(y * scale);
        const bottom = Math.max(top + 1, Math.ceil((y + 1) * scale));
        for (let x = 0; x < size; x++) {
            const left = Math.floor(x * scale);
            const right = Math.max(left + 1, Math.ceil((x + 1) * scale));
            let red = 0;
            let green = 0;
            let blue = 0;
            let alpha = 0;
            let count = 0;
            for (let sy = top; sy < bottom; sy++) {
                for (let sx = left; sx < right; sx++) {
                    const index = (sy * source.width + sx) * 4;
                    const a = source.data[index + 3];
                    red += source.data[index] * a;
                    green += source.data[index + 1] * a;
                    blue += source.data[index + 2] * a;
                    alpha += a;
                    count++;
                }
            }
            const target = (y * size + x) * 4;
            if (alpha > 0) {
                data[target] = Math.round(red / alpha);
                data[target + 1] = Math.round(green / alpha);
                data[target + 2] = Math.round(blue / alpha);
                data[target + 3] = Math.round(alpha / count);
            }
        }
    }
    return { width: size, height: size, data };
}

function encode(image) {
    return PNG.sync.write(image);
}

function buildIco(source) {
    const entries = ICO_SIZES.filter((size) => size <= source.width).map((size) => ({
        size,
        png: encode(resize(source, size)),
    }));
    const header = Buffer.alloc(6);
    header.writeUInt16LE(0, 0);
    header.writeUInt16LE(1, 2);
    header.writeUInt16LE(entries.length, 4);
    const directory = Buffer.alloc(entries.length * 16);
    let offset = 6 + directory.length;
    entries.forEach((entry, index) => {
        const at = index * 16;
        directory[at] = entry.size === 256 ? 0 : entry.size;
        directory[at + 1] = entry.size === 256 ? 0 : entry.size;
        directory[at + 2] = 0;
        directory[at + 3] = 0;
        directory.writeUInt16LE(1, at + 4);
        directory.writeUInt16LE(32, at + 6);
        directory.writeUInt32LE(entry.png.length, at + 8);
        directory.writeUInt32LE(offset, at + 12);
        offset += entry.png.length;
    });
    return Buffer.concat([header, directory, ...entries.map((entry) => entry.png)]);
}

function buildIcns(source) {
    const cache = new Map();
    const chunks = ICNS_TYPES.filter(([, size]) => size <= source.width).map(([type, size]) => {
        if (!cache.has(size)) cache.set(size, encode(resize(source, size)));
        const png = cache.get(size);
        const chunk = Buffer.alloc(8);
        chunk.write(type, 0, "ascii");
        chunk.writeUInt32BE(png.length + 8, 4);
        return Buffer.concat([chunk, png]);
    });
    const body = Buffer.concat(chunks);
    const header = Buffer.alloc(8);
    header.write("icns", 0, "ascii");
    header.writeUInt32BE(body.length + 8, 4);
    return Buffer.concat([header, body]);
}

export function patchWindowsIcon(hostBytes, icoBytes) {
    const exe = NtExecutable.from(hostBytes, { ignoreCert: true });
    const resources = NtExecutableResource.from(exe);
    const icons = Data.IconFile.from(icoBytes);
    Resource.IconGroupEntry.replaceIconsForResource(
        resources.entries,
        1,
        1033,
        icons.icons.map((icon) => icon.data),
    );
    resources.outputResource(exe);
    return Buffer.from(exe.generate());
}
