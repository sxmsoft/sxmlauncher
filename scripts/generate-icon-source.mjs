#!/usr/bin/env node
/**
 * Generates `assets/icon-source.png`, the 1024×1024 master image that
 * `pnpm tauri icon` expands into every platform icon.
 *
 * Written by hand (raw RGBA + zlib deflate) instead of pulling an image library:
 * this is the only asset pipeline step in the repo, and a 60-line script with no
 * dependencies is easier to audit than a native dependency tree.
 *
 * Run with: node scripts/generate-icon-source.mjs
 */

import { deflateSync } from "node:zlib";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const SIZE = 1024;
const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

/** CRC-32 table for PNG chunks. */
const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    table[n] = c >>> 0;
  }
  return table;
})();

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc = CRC_TABLE[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length, 0);
  const typeBuffer = Buffer.from(type, "ascii");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([typeBuffer, data])), 0);
  return Buffer.concat([length, typeBuffer, data, crc]);
}

/**
 * Paints the icon: a slate-to-violet diagonal gradient with a rounded inner
 * panel and a stylised "SX" mark, all drawn with plain maths (no canvas).
 */
function paint() {
  const raw = Buffer.alloc(SIZE * (SIZE * 4 + 1));
  const centre = SIZE / 2;
  const radius = SIZE * 0.42;
  const stroke = SIZE * 0.045;

  // Distance from a point to a line segment, used for the monogram strokes.
  const distanceToSegment = (px, py, x1, y1, x2, y2) => {
    const dx = x2 - x1;
    const dy = y2 - y1;
    const lengthSquared = dx * dx + dy * dy;
    const t = lengthSquared === 0 ? 0 : Math.max(0, Math.min(1, ((px - x1) * dx + (py - y1) * dy) / lengthSquared));
    const cx = x1 + t * dx;
    const cy = y1 + t * dy;
    return Math.hypot(px - cx, py - cy);
  };

  for (let y = 0; y < SIZE; y += 1) {
    const rowStart = y * (SIZE * 4 + 1);
    raw[rowStart] = 0; // PNG filter: none
    for (let x = 0; x < SIZE; x += 1) {
      const offset = rowStart + 1 + x * 4;
      const t = (x + y) / (SIZE * 2);

      // Background gradient (slate 900 -> violet 600).
      let r = Math.round(15 + t * (124 - 15));
      let g = Math.round(23 + t * (58 - 23));
      let b = Math.round(42 + t * (237 - 42));
      let a = 255;

      // Rounded-square mask so the icon looks right in a dock/taskbar.
      const dx = Math.abs(x - centre) / radius;
      const dy = Math.abs(y - centre) / radius;
      const inside = Math.pow(dx, 6) + Math.pow(dy, 6) <= 1.0;
      if (!inside) {
        a = 0;
      } else {
        // Inner vignette panel.
        const panel = Math.pow(dx, 3) + Math.pow(dy, 3) <= 0.72;
        if (panel) {
          r = Math.round(r * 0.72);
          g = Math.round(g * 0.72);
          b = Math.round(b * 0.78);
        }

        // Monogram: "S" (two arcs approximated by segments) + "X".
        const scale = SIZE / 32;
        const s1 = distanceToSegment(
          x, y,
          9.5 * scale, 11 * scale,
          15.5 * scale, 11 * scale,
        );
        const s2 = distanceToSegment(x, y, 9.5 * scale, 11 * scale, 9.5 * scale, 16 * scale);
        const s3 = distanceToSegment(x, y, 9.5 * scale, 16 * scale, 15.5 * scale, 16 * scale);
        const s4 = distanceToSegment(x, y, 15.5 * scale, 16 * scale, 15.5 * scale, 21 * scale);
        const s5 = distanceToSegment(x, y, 9.5 * scale, 21 * scale, 15.5 * scale, 21 * scale);

        const x1 = distanceToSegment(x, y, 18.5 * scale, 11 * scale, 24.5 * scale, 21 * scale);
        const x2 = distanceToSegment(x, y, 24.5 * scale, 11 * scale, 18.5 * scale, 21 * scale);

        const mark = Math.min(s1, s2, s3, s4, s5, x1, x2);
        if (mark <= stroke) {
          const edge = Math.max(0, Math.min(1, (stroke - mark) / (stroke * 0.5)));
          r = Math.round(r + (241 - r) * edge);
          g = Math.round(g + (245 - g) * edge);
          b = Math.round(b + (249 - b) * edge);
        }
      }

      raw[offset] = r;
      raw[offset + 1] = g;
      raw[offset + 2] = b;
      raw[offset + 3] = a;
    }
  }
  return raw;
}

function png(raw) {
  const signature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(SIZE, 0);
  ihdr.writeUInt32BE(SIZE, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // colour type: RGBA
  ihdr[10] = 0; // deflate
  ihdr[11] = 0; // adaptive filtering
  ihdr[12] = 0; // no interlace

  return Buffer.concat([
    signature,
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

const target = join(ROOT, "assets", "icon-source.png");
mkdirSync(dirname(target), { recursive: true });
writeFileSync(target, png(paint()));
console.log(`wrote ${target}`);
