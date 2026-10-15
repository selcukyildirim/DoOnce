/** Writes simple PNG toolbar icons (no dependencies). */
import { writeFileSync, mkdirSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import zlib from "node:zlib";

const out = resolve(dirname(fileURLToPath(import.meta.url)), "../icons");
mkdirSync(out, { recursive: true });

function png(size, r, g, b) {
  const row = Buffer.alloc(1 + size * 3);
  const raw = [];
  for (let y = 0; y < size; y++) {
    row[0] = 0;
    for (let x = 0; x < size; x++) {
      const cx = x - size / 2;
      const cy = y - size / 2;
      const inCircle = cx * cx + cy * cy <= (size * 0.42) ** 2;
      const i = 1 + x * 3;
      if (inCircle) {
        row[i] = r;
        row[i + 1] = g;
        row[i + 2] = b;
      } else {
        row[i] = 245;
        row[i + 1] = 245;
        row[i + 2] = 247;
      }
    }
    raw.push(row);
  }
  const compressed = zlib.deflateSync(Buffer.concat(raw));
  const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8;
  ihdr[9] = 2;
  const chunk = (type, data) => {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const t = Buffer.from(type);
    const crc = crc32(Buffer.concat([t, data]));
    const c = Buffer.alloc(4);
    c.writeUInt32BE(crc >>> 0);
    return Buffer.concat([len, t, data, c]);
  };
  return Buffer.concat([
    signature,
    chunk("IHDR", ihdr),
    chunk("IDAT", compressed),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) {
    c ^= buf[i];
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  }
  return (c ^ 0xffffffff) >>> 0;
}

for (const size of [16, 48, 128]) {
  const path = resolve(out, `icon-${size}.png`);
  writeFileSync(path, png(size, 0, 113, 227)); // #0071e3
  console.log("wrote", path);
}
