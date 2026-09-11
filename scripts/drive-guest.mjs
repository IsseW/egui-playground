// Drives a wasip1 guest module through 60 frames and checks that it paints.
//
// Usage: node --experimental-wasi-unstable-preview1 scripts/drive-guest.mjs <guest.wasm>
//
// This is the same call sequence the browser runtime worker uses: run `_start`, then for each
// frame `pg_alloc`, write the encoded input, `pg_frame`, read the length prefix and payload.

import { readFile } from "node:fs/promises";
import { WASI } from "node:wasi";

const MAGIC = 0x42475045; // "EPGB" little-endian
const VERSION = 1;

class Cursor {
  constructor(view) {
    this.view = view;
    this.pos = 0;
  }
  u8() {
    return this.view.getUint8(this.pos++);
  }
  u32() {
    const v = this.view.getUint32(this.pos, true);
    this.pos += 4;
    return v;
  }
  u64() {
    const v = this.view.getBigUint64(this.pos, true);
    this.pos += 8;
    return v;
  }
  f32() {
    const v = this.view.getFloat32(this.pos, true);
    this.pos += 4;
    return v;
  }
  skip(n) {
    this.pos += n;
  }
  option(read) {
    return this.u8() === 0 ? null : read();
  }
}

function encodeInput({ width, height, pixelsPerPoint, time, events }) {
  const parts = [];
  const header = new DataView(new ArrayBuffer(47));
  let at = 0;
  header.setUint32(at, MAGIC, true);
  at += 4;
  header.setUint16(at, VERSION, true);
  at += 2;
  for (const v of [0, 0, width, height]) {
    header.setFloat32(at, v, true);
    at += 4;
  }
  header.setFloat32(at, pixelsPerPoint, true);
  at += 4;
  header.setFloat64(at, time, true);
  at += 8;
  header.setFloat32(at, 1 / 60, true);
  at += 4;
  header.setUint8(at, 1); // focused
  at += 1;
  header.setUint32(at, 8192, true); // max_texture_side
  at += 4;
  header.setUint32(at, events.length, true);
  at += 4;
  parts.push(new Uint8Array(header.buffer));

  for (const event of events) {
    parts.push(encodeEvent(event));
  }

  const total = parts.reduce((sum, part) => sum + part.length, 0);
  const out = new Uint8Array(total);
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}

function encodeEvent(event) {
  if (event.kind === "pointer_moved") {
    const buf = new DataView(new ArrayBuffer(9));
    buf.setUint8(0, 6);
    buf.setFloat32(1, event.x, true);
    buf.setFloat32(5, event.y, true);
    return new Uint8Array(buf.buffer);
  }
  if (event.kind === "pointer_button") {
    const buf = new DataView(new ArrayBuffer(12));
    buf.setUint8(0, 8);
    buf.setFloat32(1, event.x, true);
    buf.setFloat32(5, event.y, true);
    buf.setUint8(9, 0); // primary
    buf.setUint8(10, event.pressed ? 1 : 0);
    buf.setUint8(11, 0); // no modifiers
    return new Uint8Array(buf.buffer);
  }
  throw new Error(`unknown event ${event.kind}`);
}

function decodeOutput(bytes) {
  const c = new Cursor(new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength));
  if (c.u32() !== MAGIC) {
    throw new Error("output did not start with the bridge magic");
  }
  const version = c.view.getUint16(c.pos, true);
  c.skip(2);
  if (version !== VERSION) {
    throw new Error(`output bridge version ${version}, expected ${VERSION}`);
  }

  const pixelsPerPoint = c.f32();

  const primitives = [];
  const primitiveCount = c.u32();
  for (let i = 0; i < primitiveCount; i++) {
    c.skip(16); // clip rect
    c.skip(1);
    c.u64(); // texture id
    const indices = c.u32();
    c.skip(indices * 4);
    const vertices = c.u32();
    c.skip(vertices * 20);
    primitives.push({ indices, vertices });
  }

  const texturesSet = [];
  const setCount = c.u32();
  for (let i = 0; i < setCount; i++) {
    c.skip(1);
    const id = c.u64();
    const deltaCount = c.u32();
    for (let d = 0; d < deltaCount; d++) {
      const size = [c.u32(), c.u32()];
      c.skip(8); // source size
      const pixels = c.u32();
      c.skip(pixels * 4);
      c.skip(3); // magnification, minification, wrap mode
      c.option(() => c.skip(1)); // mipmap mode
      const pos = c.option(() => [c.u32(), c.u32()]);
      texturesSet.push({ id, size, pos });
    }
  }

  const texturesFree = [];
  const freeCount = c.u32();
  for (let i = 0; i < freeCount; i++) {
    c.skip(1);
    texturesFree.push(c.u64());
  }

  return { pixelsPerPoint, primitives, texturesSet, texturesFree };
}

async function main() {
  const path = process.argv[2];
  if (!path) {
    console.error("usage: drive-guest.mjs <guest.wasm>");
    process.exit(2);
  }

  const wasi = new WASI({ version: "preview1", args: [path], returnOnExit: true });

  // Under `wasm32-wasip1` egui still pulls in `web-sys`, whose imports are never called.
  const placeholder = new Proxy({}, { get: () => () => 0 });
  const imports = {
    ...wasi.getImportObject(),
    __wbindgen_placeholder__: placeholder,
    __wbindgen_externref_xform__: placeholder,
  };

  const wasm = await WebAssembly.compile(await readFile(path));
  const instance = await WebAssembly.instantiate(wasm, imports);
  wasi.start(instance);

  const { memory, pg_alloc, pg_frame } = instance.exports;

  let firstFontTexture = null;
  let painted = 0;
  const started = performance.now();

  for (let frame = 0; frame < 60; frame++) {
    const events = [];
    if (frame === 10) {
      events.push({ kind: "pointer_moved", x: 40, y: 60 });
    }
    if (frame === 11) {
      events.push({ kind: "pointer_button", x: 40, y: 60, pressed: true });
    }
    if (frame === 12) {
      events.push({ kind: "pointer_button", x: 40, y: 60, pressed: false });
    }

    const input = encodeInput({
      width: 640,
      height: 480,
      pixelsPerPoint: 2,
      time: frame / 60,
      events,
    });

    const ptr = pg_alloc(input.length);
    new Uint8Array(memory.buffer).set(input, ptr);
    const resultPtr = pg_frame(ptr, input.length);

    const bytes = new Uint8Array(memory.buffer);
    const len = new DataView(memory.buffer).getUint32(resultPtr, true);
    const output = decodeOutput(bytes.slice(resultPtr + 4, resultPtr + 4 + len));

    const vertices = output.primitives.reduce((sum, p) => sum + p.vertices, 0);
    if (vertices > 0) {
      painted++;
    }
    if (frame === 0) {
      firstFontTexture = output.texturesSet.find((t) => t.id === 0n) ?? null;
      console.log(
        `frame 0: ${output.primitives.length} primitives, ${vertices} vertices, ` +
          `${output.texturesSet.length} texture uploads`
      );
    }
  }

  const elapsed = performance.now() - started;

  const failures = [];
  if (painted !== 60) {
    failures.push(`only ${painted} of 60 frames produced vertices`);
  }
  if (!firstFontTexture) {
    failures.push("frame 0 did not upload the font atlas");
  }

  if (failures.length > 0) {
    for (const failure of failures) {
      console.error(`FAIL: ${failure}`);
    }
    process.exit(1);
  }

  console.log(
    `ok: 60 frames in ${elapsed.toFixed(1)} ms, font atlas ${firstFontTexture.size.join("x")}`
  );
}

await main();
