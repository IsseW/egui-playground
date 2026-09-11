// Runs `rustc.wasm` on a dedicated worker: builds a program to wasm, or type-checks it.
//
// Ported from weblings (MIT): https://github.com/AngelOnFira/weblings, `app/public/worker.js`.
//
// Compiling lives here, off the main thread, for two reasons: a compile never blocks typing or
// rendering, and synchronous wasm cannot be interrupted but a worker can be killed mid-compile
// with `terminate()`, which is how `runner.js` drops a superseded compile.
//
// The built module goes back to the caller. Running it is the runtime worker's job.
//
// Messages in:  {type:"init", module, bundleUrls, bundles?}  {type:"job", id, kind, source}
// Messages out: {type:"ready"}  {type:"init-error", error}
//               {type:"status", id, text}  {type:"result", id, result, wasm?}

import { Fd, File, Directory, PreopenDirectory, WASI } from "./vendor/browser_wasi_shim/index.js";

// Where both bundles place their rlibs, and where `--sysroot /sysroot` looks for them.
const LIB_DIR = "lib/rustlib/wasm32-wasip1/lib";

// The crates a program links against by name, in the order they reach `--extern`.
const EXTERN_CRATES = ["egui", "egui_playground"];

// Linked in through `egui_playground`, so it only has to be present.
const REQUIRED_CRATES = ["pg_abi"];

let rustcModule = null;

// The merged sysroot: `Map(relative path -> File)`, the egui bundle laid over the std one.
let sysroot = null;

// `Map(crate name -> path under /sysroot)` for the crates a program links against.
let externs = null;

// Parses the RIWB1 bundle format: "RIWB1\n", a u32le index length, a JSON index
// `{files: [{p, o, l}], total}`, then the concatenated file bytes.
function parseBundle(bytes) {
    const magic = new TextDecoder().decode(bytes.subarray(0, 6));
    if (magic !== "RIWB1\n") throw new Error("bad sysroot bundle magic");
    const indexLength = new DataView(bytes.buffer, bytes.byteOffset + 6, 4).getUint32(0, true);
    const index = JSON.parse(new TextDecoder().decode(bytes.subarray(10, 10 + indexLength)));
    const base = 10 + indexLength;
    const files = new Map();
    for (const file of index.files) {
        if (file.p === "manifest.json") continue;
        files.set(file.p, new File(bytes.slice(base + file.o, base + file.o + file.l)));
    }
    return files;
}

// Finds `lib<name>-<hash>.rlib` in the merged tree. The hashes change with every sysroot build,
// so the filenames are read off the tree rather than written down.
function findRlib(name) {
    const prefix = `${LIB_DIR}/lib${name}-`;
    for (const path of sysroot.keys()) {
        if (path.startsWith(prefix) && path.endsWith(".rlib")) return path;
    }
    return null;
}

// Builds the `/sysroot` preopen from the flat paths of the merged tree.
function sysrootPreopen() {
    const root = new Map();
    const dirFor = (segments) => {
        let map = root;
        for (const segment of segments) {
            if (!map.has(segment)) map.set(segment, new Map());
            map = map.get(segment);
        }
        return map;
    };
    for (const [path, file] of sysroot) {
        const segments = path.split("/");
        const name = segments.pop();
        dirFor(segments).set(name, file);
    }
    const toDir = (map) =>
        new Directory([...map.entries()].map(([n, v]) => [n, v instanceof Map ? toDir(v) : v]));
    return new PreopenDirectory(
        "/sysroot",
        [...root.entries()].map(([n, v]) => [n, v instanceof Map ? toDir(v) : v]),
    );
}

async function bundleBytes(url, transferred) {
    if (transferred) return new Uint8Array(transferred);
    if (typeof caches !== "undefined") {
        // The preload staged the bundle here so worker spawns read it off the main thread.
        try {
            const hit = await caches.match(url);
            if (hit) return new Uint8Array(await hit.arrayBuffer());
        } catch {
            // Fall through to the network, which hits the browser's HTTP cache.
        }
    }
    const response = await fetch(url);
    if (!response.ok) throw new Error(`fetch ${url}: ${response.status}`);
    return new Uint8Array(await response.arrayBuffer());
}

async function initEngine(message) {
    rustcModule = message.module;

    // Both bundles hold paths under `lib/rustlib/wasm32-wasip1/lib`, so merging them is an
    // overlay of the two file maps, the egui one last.
    sysroot = new Map();
    for (const [i, url] of message.bundleUrls.entries()) {
        const bytes = await bundleBytes(url, message.bundles && message.bundles[i]);
        for (const [path, file] of parseBundle(bytes)) sysroot.set(path, file);
    }

    externs = new Map();
    const missing = [];
    for (const name of [...EXTERN_CRATES, ...REQUIRED_CRATES]) {
        const path = findRlib(name);
        if (path) {
            externs.set(name, `/sysroot/${path}`);
        } else {
            missing.push(name);
        }
    }
    if (missing.length) throw new Error(`sysroot has no rlib for ${missing.join(", ")}`);
}

// Collects everything rustc writes to a stream.
function capture() {
    const decoder = new TextDecoder();
    const state = { text: "" };
    state.Fd = class extends Fd {
        fd_write(data) {
            state.text += decoder.decode(data, { stream: true });
            return { ret: 0, nwritten: data.byteLength };
        }
    };
    return state;
}

const ANSI = /\x1b\[[0-9;]*m/g;

// rustc writes one JSON diagnostic per stderr line under `--error-format=json`. The structured
// list feeds the editor squiggles, and the `rendered` texts reconstruct the terminal output.
function parseDiagnostics(log) {
    const out = [];
    for (const line of log.split("\n")) {
        if (line[0] !== "{") continue;
        let d;
        try {
            d = JSON.parse(line);
        } catch {
            continue;
        }
        if (d.$message_type !== "diagnostic") continue;
        if (d.level !== "error" && d.level !== "warning") continue;
        const spans = d.spans || [];
        const span = spans.find((s) => s.is_primary) || spans[0] || null;
        const ansi = (d.rendered || d.message).trimEnd();
        out.push({
            level: d.level,
            message: d.message,
            code: d.code && d.code.code ? d.code.code : null,
            line: span ? span.line_start : null,
            col: span ? span.column_start : null,
            endLine: span ? span.line_end : null,
            endCol: span ? span.column_end : null,
            rendered: ansi.replace(ANSI, ""),
            ansi,
        });
    }
    return out;
}

// Points spans and panic locations at the name the editor shows instead of the path inside the
// worker's file system.
function remap(text) {
    return text.replaceAll("/work/prog.rs", "program");
}

function diagnosticsOf(log) {
    return parseDiagnostics(log).map((d) => ({
        ...d,
        rendered: remap(d.rendered),
        ansi: remap(d.ansi),
    }));
}

// The args every invocation shares. `-Zallow-missing-proc-macros` is a flag of the rustc fork:
// rustc inside wasm cannot load proc-macro dylibs, and without it egui fails to load with
// `can't find crate for bytemuck_derive`.
function commonArgs() {
    return [
        "rustc",
        "/work/prog.rs",
        "--sysroot",
        "/sysroot",
        "--target",
        "wasm32-wasip1",
        "--edition",
        "2024",
        "-Zunstable-options",
        "-Zallow-missing-proc-macros",
        ...EXTERN_CRATES.flatMap((name) => ["--extern", `${name}=${externs.get(name)}`]),
        "--error-format=json",
        "--json=diagnostic-rendered-ansi",
    ];
}

// Runs one rustc invocation over `/work`, which holds the program source.
async function runRustc(source, args, env) {
    const log = capture();
    const work = new PreopenDirectory("/work", [
        ["prog.rs", new File(new TextEncoder().encode(source))],
    ]);
    const fds = [
        new log.Fd(),
        new log.Fd(),
        new log.Fd(),
        new PreopenDirectory("/tmp", []),
        sysrootPreopen(),
        work,
    ];

    const enter = performance.now();
    const wasi = new WASI(args, env, fds, { debug: false });
    const setupMs = performance.now() - enter;

    const instantiateStart = performance.now();
    const instance = await WebAssembly.instantiate(rustcModule, {
        wasi_snapshot_preview1: wasi.wasiImport,
    });
    const instantiateMs = performance.now() - instantiateStart;

    const start = performance.now();
    let exit = 0;
    try {
        exit = wasi.start(instance);
    } catch (error) {
        // rustc aborts through a wasm trap after printing its diagnostics.
        const text = (error && error.message) || String(error);
        if (!log.text.trim()) log.text += text;
        exit = 1;
    }
    const runMs = performance.now() - start;

    return { work, exit, log: log.text, setupMs, instantiateMs, runMs };
}

// riwl prints one timing line to stderr under `RIWL_TIMINGS=1`:
// "riwl-timing: total_ms=N load_ms=N resolve_ms=N layout_ms=N apply_ms=N emit_ms=N".
// It splits compile time from link time, and does not belong in the user's diagnostics.
function takeLinkTimings(run) {
    const match = run.log.match(/riwl-timing: ([^\n]+)/);
    if (!match) return null;
    const phases = Object.fromEntries(
        match[1]
            .trim()
            .split(/\s+/)
            .map((pair) => pair.split("="))
            .map(([k, v]) => [k, Number(v)]),
    );
    run.log = run.log.replace(/^.*riwl-timing:[^\n]*\n?/m, "");
    return phases;
}

// ---- build: compile and link one program to wasm ----------------------------

async function buildJob(message, status) {
    status("compiling");
    const args = commonArgs();
    args.push("-O", "-Cpanic=abort", "-o", "/work/prog.wasm");
    const run = await runRustc(message.source, args, ["CLIF2WASM_OBJECT=1", "RIWL_TIMINGS=1"]);

    const linkPhases = takeLinkTimings(run);
    const linkMs = linkPhases ? linkPhases.total_ms : null;
    const compileMs = linkMs != null ? Math.max(0, run.runMs - linkMs) : run.runMs;
    const diagnostics = diagnosticsOf(run.log);
    const stages = {
        setupMs: +run.setupMs.toFixed(1),
        rustcInstantiateMs: +run.instantiateMs.toFixed(1),
        compileMs: +compileMs.toFixed(1),
        linkMs,
        totalMs: +run.runMs.toFixed(1),
    };

    const binary = run.work.dir.contents.get("prog.wasm");
    if (!binary || !binary.data || binary.data.length === 0) {
        // The host renders `diagnostics` itself. `stderr` carries the rest, which is the raw
        // output for an ICE or a trap that produced no JSON diagnostic at all.
        const residue = diagnostics.length
            ? ""
            : remap(run.log).trim() || `rustc exited ${run.exit} without emitting a program`;
        return {
            result: {
                ok: false,
                compileFailed: true,
                diagnostics,
                stderr: residue,
                exit: run.exit,
                compileMs,
                linkMs,
                linkPhases,
                stages,
            },
        };
    }

    const wasm = binary.data.slice().buffer;
    return {
        result: {
            ok: true,
            diagnostics,
            stderr: "",
            exit: run.exit,
            compileMs,
            linkMs,
            linkPhases,
            stages,
            wasmLen: wasm.byteLength,
        },
        wasm,
    };
}

// ---- check: type-check only, no codegen -------------------------------------

async function checkJob(message, status) {
    status("checking");
    const args = commonArgs();
    args.push("--emit", "metadata", "-o", "/work/libprog.rmeta");
    const run = await runRustc(message.source, args, []);

    const diagnostics = diagnosticsOf(run.log);
    const errorCount = diagnostics.filter((d) => d.level === "error").length;
    const warningCount = diagnostics.length - errorCount;
    return {
        result: {
            ok: run.exit === 0 && errorCount === 0,
            errorCount,
            warningCount,
            compileMs: +run.runMs.toFixed(1),
            exit: run.exit,
            stderr: diagnostics.length ? "" : remap(run.log).trim(),
            diagnostics,
        },
    };
}

const jobs = { build: buildJob, check: checkJob };

self.onmessage = async (event) => {
    const message = event.data;

    if (message.type === "init") {
        try {
            await initEngine(message);
            self.postMessage({ type: "ready" });
        } catch (error) {
            self.postMessage({
                type: "init-error",
                error: String((error && error.message) || error),
            });
        }
        return;
    }

    if (message.type === "job") {
        const status = (text) => self.postMessage({ type: "status", id: message.id, text });
        let done;
        try {
            done = await jobs[message.kind](message, status);
        } catch (error) {
            done = {
                result: {
                    ok: false,
                    stderr: "compiler error: " + String((error && error.message) || error),
                    diagnostics: [],
                },
            };
        }
        const reply = { type: "result", id: message.id, result: done.result };
        if (done.wasm) {
            reply.wasm = done.wasm;
            self.postMessage(reply, [done.wasm]);
        } else {
            self.postMessage(reply);
        }
    }
};
