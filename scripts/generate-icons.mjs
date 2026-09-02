// Generates placeholder app icons for Tauri (PNG + Windows .ico + macOS .icns).
// Pure Node (zlib only) — no native deps. Replace later with real branding via
// `npm run tauri icon path/to/logo.png`.
import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const ICON_DIR = join(dirname(fileURLToPath(import.meta.url)), "..", "apps", "desktop", "src-tauri", "icons");
mkdirSync(ICON_DIR, { recursive: true });

// ---- CRC32 (for PNG chunks) --------------------------------------------------
const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();
function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

// ---- pixel design ------------------------------------------------------------
function pixel(x, y, size) {
  const m = Math.floor(size * 0.12);
  let rgb = [0x1e, 0x1e, 0x2e]; // background
  if (x >= m && x < size - m && y >= m && y < size - m) rgb = [0x89, 0xb4, 0xfa]; // accent tile
  const inset = Math.floor(m * 1.6);
  const h = Math.max(1, Math.floor(size * 0.06));
  const s1 = Math.floor(size * 0.4);
  const s2 = Math.floor(size * 0.58);
  if (x >= inset && x < size - inset && ((y >= s1 && y < s1 + h) || (y >= s2 && y < s2 + h))) {
    rgb = [0x1e, 0x1e, 0x2e]; // dark "byte row" slits
  }
  return [rgb[0], rgb[1], rgb[2], 255];
}

function chunk(type, data) {
  const typeBuf = Buffer.from(type, "ascii");
  const lenBuf = Buffer.alloc(4);
  lenBuf.writeUInt32BE(data.length, 0);
  const crcBuf = Buffer.alloc(4);
  crcBuf.writeUInt32BE(crc32(Buffer.concat([typeBuf, data])), 0);
  return Buffer.concat([lenBuf, typeBuf, data, crcBuf]);
}

function makePNG(size) {
  // Raw image: each scanline is a 0 filter byte + RGBA pixels.
  const raw = Buffer.alloc(size * (1 + size * 4));
  let p = 0;
  for (let y = 0; y < size; y++) {
    raw[p++] = 0; // filter: none
    for (let x = 0; x < size; x++) {
      const [r, g, b, a] = pixel(x, y, size);
      raw[p++] = r;
      raw[p++] = g;
      raw[p++] = b;
      raw[p++] = a;
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // color type RGBA
  // 10,11,12 = compression, filter, interlace = 0
  const sig = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  return Buffer.concat([sig, chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]);
}

// ---- Windows .ico (embeds PNGs) ---------------------------------------------
function makeICO(sizes) {
  const imgs = sizes.map((s) => ({ s, png: makePNG(s) }));
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(imgs.length, 4);
  const dir = Buffer.alloc(16 * imgs.length);
  let offset = 6 + dir.length;
  imgs.forEach((img, i) => {
    const o = i * 16;
    dir[o] = img.s >= 256 ? 0 : img.s; // width (0 => 256)
    dir[o + 1] = img.s >= 256 ? 0 : img.s; // height
    dir[o + 2] = 0; // palette
    dir[o + 3] = 0; // reserved
    dir.writeUInt16LE(1, o + 4); // color planes
    dir.writeUInt16LE(32, o + 6); // bpp
    dir.writeUInt32LE(img.png.length, o + 8);
    dir.writeUInt32LE(offset, o + 12);
    offset += img.png.length;
  });
  return Buffer.concat([header, dir, ...imgs.map((i) => i.png)]);
}

// ---- macOS .icns (embeds PNGs) ----------------------------------------------
function makeICNS(entries) {
  // entries: [{ type: 'ic07', size }]
  const blocks = entries.map(({ type, size }) => {
    const png = makePNG(size);
    const b = Buffer.alloc(8 + png.length);
    b.write(type, 0, "ascii");
    b.writeUInt32BE(png.length + 8, 4);
    png.copy(b, 8);
    return b;
  });
  const body = Buffer.concat(blocks);
  const header = Buffer.alloc(8);
  header.write("icns", 0, "ascii");
  header.writeUInt32BE(body.length + 8, 4);
  return Buffer.concat([header, body]);
}

// ---- write everything --------------------------------------------------------
const write = (name, buf) => {
  writeFileSync(join(ICON_DIR, name), buf);
  console.log("  wrote", name, `(${buf.length} bytes)`);
};

console.log("Generating icons ->", ICON_DIR);
write("32x32.png", makePNG(32));
write("128x128.png", makePNG(128));
write("128x128@2x.png", makePNG(256));
write("icon.png", makePNG(512));
write("Square30x30Logo.png", makePNG(30));
write("Square44x44Logo.png", makePNG(44));
write("Square89x89Logo.png", makePNG(89));
write("Square107x107Logo.png", makePNG(107));
write("Square142x142Logo.png", makePNG(142));
write("Square150x150Logo.png", makePNG(150));
write("Square284x284Logo.png", makePNG(284));
write("Square310x310Logo.png", makePNG(310));
write("StoreLogo.png", makePNG(50));
write("icon.ico", makeICO([16, 32, 48, 64, 256]));
write(
  "icon.icns",
  makeICNS([
    { type: "ic07", size: 128 },
    { type: "ic08", size: 256 },
    { type: "ic09", size: 512 },
    { type: "ic11", size: 32 },
    { type: "ic12", size: 64 },
  ])
);
console.log("Done.");
