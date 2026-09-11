# egui playground

A playground for egui!

The page is one host egui app, that has an editor and draws meshes from a guest egui app from a separete wasm blob.

That wasm blob is compiled by rustc & cranelift in the browser.

## Crates

| crate | what it is |
| --- | --- |
| `pg-abi` | binary encoding of frame input and frame output. Linked by both sides. |
| `egui_playground` | the prelude a guest program links against. `App`, `run`, and the exported entry points. |
| `host` | the eframe web app: editor, guest view, worker glue. |
| `guest-examples` | example programs, built for `wasm32-wasip1`. |

## Guest ABI

A guest is a `wasm32-wasip1` binary. `main` calls `egui_playground::run(app)`, which keeps the app
and an `egui::Context` in a thread local and returns. The host then drives frames:

- `pg_alloc(len) -> *mut u8` reserves a buffer for the frame input.
- `pg_frame(ptr, len) -> *const u8` runs one frame and returns a `u32` byte count followed by that
  many bytes of encoded output. It stays valid until the next call.
- `pg_free(ptr, len)` releases a buffer that was never passed to `pg_frame`.

Thread-local state survives between calls, so the app keeps its state across frames.

## Running the examples

```sh
scripts/build-examples.sh                  # build the guests and stage them for the host
cd crates/host && trunk serve              # the playground itself
```

## Verification

```sh
cargo test
node scripts/drive-guest.mjs target/wasm32-wasip1/release/hello.wasm
```

`scripts/drive-guest.mjs` makes the same calls the browser runtime worker makes, so it catches
bridge breakage without a browser.

## Attribution

Browser compilation was more or less copied from [weblings](https://github.com/AngelOnFira/weblings)

(MIT): bjorn3's rustc-in-wasm fork, `clif2wasm`, and the `riwl` linker, with `browser_wasi_shim` in the worker.
