# egui playground

A Rust-playground-style site for egui. Code editor on the left, the running egui program on the
right, compiled in the browser.

The whole page is one eframe app. The user's program is a separate `wasm32-wasip1` module that
never touches a canvas: it produces meshes and texture uploads, the host renders them into an
offscreen texture and shows it as an image.

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
cargo test                                                      # bridge round trips, 60 frames natively
node scripts/drive-guest.mjs target/wasm32-wasip1/release/hello.wasm
```

`scripts/drive-guest.mjs` makes the same calls the browser runtime worker makes, so it catches
bridge breakage without a browser.

## Numbers

Measured on an M-series mac, egui 0.36, release guests.

| what | value |
| --- | --- |
| guest module size | 4.9 MB (`hello`), 5.1 MB (`widgets`) |
| 60 frames under node, 640x480 at 2x | 21 ms (`hello`), 29 ms (`widgets`) |
| first frame font atlas | 8192x32 RGBA, 1 MB |

Most of the module size is the default font data. Dropping `default_fonts` and having the host
supply font bytes is the lever if that matters.

## Reuse

Browser compilation follows [weblings](https://github.com/AngelOnFira/weblings) (MIT): bjorn3's
rustc-in-wasm fork, `clif2wasm`, and the `riwl` linker, with `browser_wasi_shim` in the worker.
