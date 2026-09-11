// The toolchain preload and the compile worker pool behind `window.compileRust` and
// `window.checkRust`.
//
// Ported from weblings (MIT): https://github.com/AngelOnFira/weblings, `app/public/runner.js`.
//
// Compiling happens in `compile-worker.js` on a dedicated worker, so a compile never blocks the
// page and an in-flight compile can be cancelled by terminating its worker.
//
//   preload: `rustc.wasm` plus the two sysroot bundles, streamed with byte progress. The bundles
//     are staged in the Cache API so a respawned worker reads them on its own thread, and the
//     rustc module travels to workers by structured clone.
//   pool: a couple of warm workers, module compiled and sysroot parsed. One job runs at a time,
//     and the newest submission wins.
//   API: `window.compileRust(source, status)` and `window.checkRust(source)` resolve to
//     `{ json, wasm }`, where `json` is the result as text for the Rust side to parse and `wasm`
//     is the built module or null. A superseded call resolves with `{"cancelled":true}`.

const TOOLCHAIN = "./rustc/";
const STD_BUNDLE_URL = new URL(TOOLCHAIN + "sysroot-wasip1.bundle", import.meta.url).href;
const EGUI_BUNDLE_URL = new URL(TOOLCHAIN + "sysroot-egui.bundle", import.meta.url).href;
const RUSTC_URL = new URL(TOOLCHAIN + "rustc.wasm", import.meta.url).href;
const META_URL = new URL(TOOLCHAIN + "assets-meta.json", import.meta.url).href;
const BUNDLE_CACHE = "egui-playground-toolchain-v1";

let rustcModule = null;

// Bundle bytes kept here only when the Cache API is unavailable, cloned to each worker at spawn.
let bundleBytes = null;

// Streams the body of `url`, reporting received bytes through `onBytes`.
async function fetchCounted(url, onBytes) {
    const response = await fetch(url);
    if (!response.ok) throw new Error(`fetch ${url}: ${response.status}`);
    const reader = response.body.getReader();
    return new ReadableStream({
        async pull(controller) {
            const { done, value } = await reader.read();
            if (done) {
                controller.close();
                return;
            }
            onBytes(value.byteLength);
            controller.enqueue(value);
        },
        cancel(reason) {
            return reader.cancel(reason);
        },
    });
}

async function streamToBytes(stream) {
    const chunks = [];
    const reader = stream.getReader();
    let total = 0;
    for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        chunks.push(value);
        total += value.byteLength;
    }
    const out = new Uint8Array(total);
    let offset = 0;
    for (const chunk of chunks) {
        out.set(chunk, offset);
        offset += chunk.byteLength;
    }
    return out;
}

// `onProgress(receivedBytes, totalBytes)`. The total comes from `assets-meta.json`, which lists
// decompressed sizes, the same unit the Streams API hands out. It is 0 when that file is missing.
let preloadPromise = null;
window.preloadRust = function (onProgress) {
    if (preloadPromise) return preloadPromise;
    preloadPromise = (async () => {
        let total = 0;
        try {
            const meta = await (await fetch(META_URL)).json();
            for (const value of Object.values(meta)) {
                if (typeof value === "number") total += value;
            }
        } catch {
            // No total means no percentage, the callback still sees the byte count.
        }

        let received = 0;
        let reported = -1;
        const onBytes = (n) => {
            received += n;
            if (!onProgress) return;
            // One report per percent, so a per-chunk callback does not flood the host.
            const step = total ? Math.floor((received / total) * 100) : Math.floor(received / 1e6);
            if (step === reported) return;
            reported = step;
            onProgress(received, total);
        };

        const [std, egui, rustc] = await Promise.all([
            fetchCounted(STD_BUNDLE_URL, onBytes).then(streamToBytes),
            fetchCounted(EGUI_BUNDLE_URL, onBytes).then(streamToBytes),
            fetchCounted(RUSTC_URL, onBytes).then((stream) =>
                WebAssembly.compileStreaming(
                    new Response(stream, { headers: { "Content-Type": "application/wasm" } }),
                ),
            ),
        ]);
        rustcModule = rustc;

        // Staging the bundles in the Cache API lets a worker read them on its own thread, so
        // cancelling never clones them through this one.
        try {
            const cache = await caches.open(BUNDLE_CACHE);
            await cache.put(STD_BUNDLE_URL, new Response(std));
            await cache.put(EGUI_BUNDLE_URL, new Response(egui));
            bundleBytes = null;
        } catch {
            bundleBytes = { [STD_BUNDLE_URL]: std, [EGUI_BUNDLE_URL]: egui };
        }

        if (onProgress) onProgress(received, total || received);
        ensurePool();
    })();
    return preloadPromise;
};

async function ensureReady(status) {
    if (rustcModule) return;
    status && status("downloading the toolchain");
    try {
        await window.preloadRust();
    } catch {
        preloadPromise = null;
        await window.preloadRust();
    }
}

// At most one job in flight in `current`, and at most one waiting in `queued`, always the newest.
// Older waiters resolve as cancelled.
const POOL_TARGET = 2;
const idle = [];
let warming = 0;
let current = null;
let queued = null;
let jobSeq = 0;

const CANCELLED = { json: '{"cancelled":true}', wasm: null };

function spawnWorker() {
    warming++;
    const handle = {
        worker: new Worker(new URL("./compile-worker.js", import.meta.url), { type: "module" }),
    };
    const init = {
        type: "init",
        module: rustcModule,
        bundleUrls: [STD_BUNDLE_URL, EGUI_BUNDLE_URL],
    };
    if (bundleBytes) {
        // Structured clone: one copy per worker, the master stays here.
        init.bundles = [
            bundleBytes[STD_BUNDLE_URL].buffer,
            bundleBytes[EGUI_BUNDLE_URL].buffer,
        ];
    }
    handle.worker.postMessage(init);
    handle.worker.onmessage = (event) => onWorkerMessage(handle, event.data);
    handle.worker.onerror = (event) =>
        console.warn("[playground] compile worker error:", event.message || event);
}

function onWorkerMessage(handle, message) {
    if (message.type === "ready") {
        warming--;
        idle.push(handle);
        dispatch();
        return;
    }
    if (message.type === "init-error") {
        warming--;
        console.warn("[playground] compile worker init failed:", message.error);
        handle.worker.terminate();
        return;
    }

    // Only the current job's worker and id are live, anything else is late traffic from a
    // superseded job.
    if (!current || current.handle !== handle || current.id !== message.id) return;

    if (message.type === "status") {
        current.statusCb && current.statusCb(message.text);
        return;
    }
    if (message.type === "result") {
        const { resolve } = current;
        current = null;
        idle.push(handle);
        resolve({
            json: JSON.stringify(message.result),
            wasm: message.wasm ? new Uint8Array(message.wasm) : null,
        });
        dispatch();
    }
}

function ensurePool() {
    if (!rustcModule) return;
    while (idle.length + warming + (current ? 1 : 0) < POOL_TARGET) spawnWorker();
}

function dispatch() {
    if (!queued || current) return;
    const handle = idle.pop();
    if (!handle) {
        ensurePool(); // A `ready` message re-enters here.
        return;
    }
    const request = queued;
    queued = null;
    current = {
        handle,
        id: ++jobSeq,
        statusCb: request.statusCb,
        resolve: request.resolve,
    };
    handle.worker.postMessage({ type: "job", id: current.id, kind: request.kind, ...request.payload });
    ensurePool(); // Keep a spare warming while this job runs.
}

// Submits a job and cancels whatever is in flight: the busy worker is terminated mid-compile and
// the superseded promise resolves as cancelled. Newest wins by submission order, taken
// synchronously, so a call parked in `ensureReady` cannot wake up late and cancel a newer one.
let submitSeq = 0;
async function submit(kind, payload, statusCb) {
    const seq = ++submitSeq;
    await ensureReady(statusCb);
    return new Promise((resolve) => {
        if (seq < submitSeq) {
            resolve(CANCELLED);
            return;
        }
        if (current) {
            current.handle.worker.terminate();
            current.resolve(CANCELLED);
            current = null;
        }
        if (queued) queued.resolve(CANCELLED);
        queued = { kind, payload, statusCb, resolve };
        dispatch();
    });
}

const asStatusCb = (status) =>
    typeof status === "function"
        ? (text) => {
              try {
                  status(text);
              } catch {
                  // The host dropped the closure.
              }
          }
        : null;

// Builds `source` into a `wasm32-wasip1` module. Resolves `{ json, wasm }`.
window.compileRust = (source, status) => submit("build", { source }, asStatusCb(status));

// Type-checks `source`. Resolves `{ json, wasm: null }`.
window.checkRust = (source, status) => submit("check", { source }, asStatusCb(status));
