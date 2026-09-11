// Runs one guest program and answers frame requests from the host.
//
// Messages in:  {type:"load", wasm}  {type:"frame", input}
// Messages out: {type:"ready"}  {type:"frame", output}  {type:"stdout"|"stderr", text}
//               {type:"trapped", message}

import {
    WASI,
    OpenFile,
    File,
    ConsoleStdout,
} from "./vendor/browser_wasi_shim/index.js";

// The guest module, or `null` while nothing is loaded and after a trap.
let guest = null;

function post(message) {
    self.postMessage(message);
}

// `wasm-bindgen` leaves this import module behind in a `wasm32-wasip1` build. Nothing in a
// playground program calls it, so every name resolves to a function that does nothing.
function placeholderImports() {
    return new Proxy(
        {},
        {
            get: () => () => {},
            has: () => true,
        },
    );
}

function imports(wasi) {
    return new Proxy(
        { wasi_snapshot_preview1: wasi.wasiImport },
        {
            get: (target, name) =>
                name in target ? target[name] : placeholderImports(),
            has: () => true,
        },
    );
}

async function load(wasmBytes) {
    guest = null;

    const fds = [
        new OpenFile(new File([])),
        ConsoleStdout.lineBuffered((text) => post({ type: "stdout", text })),
        ConsoleStdout.lineBuffered((text) => post({ type: "stderr", text })),
    ];
    const wasi = new WASI([], [], fds);

    const module = await WebAssembly.compile(wasmBytes);
    const instance = await WebAssembly.instantiate(module, imports(wasi));

    try {
        wasi.start(instance);
    } catch (error) {
        post({ type: "trapped", message: `start: ${error}` });
        return;
    }

    const { memory, pg_alloc, pg_frame } = instance.exports;
    if (!memory || !pg_alloc || !pg_frame) {
        post({
            type: "trapped",
            message: "module does not export pg_alloc, pg_frame and memory",
        });
        return;
    }

    guest = { memory, pg_alloc, pg_frame };
    post({ type: "ready" });
}

function frame(inputBytes) {
    if (!guest) {
        return;
    }

    const { memory, pg_alloc, pg_frame } = guest;
    const input = new Uint8Array(inputBytes);

    let output;
    try {
        const ptr = pg_alloc(input.length);
        new Uint8Array(memory.buffer, ptr, input.length).set(input);

        const result = pg_frame(ptr, input.length);
        const length = new DataView(memory.buffer).getUint32(result, true);
        output = new Uint8Array(
            memory.buffer.slice(result + 4, result + 4 + length),
        );
    } catch (error) {
        guest = null;
        post({ type: "trapped", message: `${error}` });
        return;
    }

    post({ type: "frame", output: output.buffer }, [output.buffer]);
}

self.onmessage = async (event) => {
    const message = event.data;
    switch (message.type) {
        case "load":
            try {
                await load(message.wasm);
            } catch (error) {
                guest = null;
                post({ type: "trapped", message: `load: ${error}` });
            }
            break;
        case "frame":
            frame(message.input);
            break;
    }
};
