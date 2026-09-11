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

Measured on an M-series mac, egui 0.36.

What a first visit downloads. GitHub Pages gzips both `application/octet-stream` and
`application/wasm`, so the gzip column is what crosses the wire.

| artifact | raw | gzip |
| --- | --- | --- |
| `rustc.wasm` | 87.9 MB | 20.0 MB |
| std sysroot bundle | 71.3 MB | 23.5 MB |
| egui sysroot bundle | 105.9 MB | 35.5 MB |
| host wasm | 8.0 MB | 3.3 MB |
| total | 273 MB | 82.3 MB |

The egui bundle is the largest single item, and 61% of an egui rlib is metadata rather than
object code, which no optimization flag reaches. `codegen-units = 1` takes the rlib set from
101 MB to 96 MB but costs 12% of guest frame time, so the rlibs stay on the `release` profile.

The host is built with the `small` profile, which is `opt-level = "z"`, `lto`, one codegen unit,
`panic = "abort"` and `strip`.

| host build | raw | gzip | brotli |
| --- | --- | --- | --- |
| `release` | 9.27 MB | 3.90 MB | 2.97 MB |
| `small` | 7.99 MB | 3.26 MB | 2.60 MB |

Compiling in the browser, per program, one `rustc` invocation against the prebuilt rlibs:

| what | value |
| --- | --- |
| compile + link, natively | 215 ms (`hello`), 249 ms (`widgets`) |
| guest module size | 7.6 MB |
| 60 frames under node, 640x480 at 2x | 21 ms (`hello`), 32 ms (`widgets`) |
| first frame font atlas | 8192x32 RGBA, 1 MB |

## Reuse

Browser compilation follows [weblings](https://github.com/AngelOnFira/weblings) (MIT): bjorn3's
rustc-in-wasm fork, `clif2wasm`, and the `riwl` linker, with `browser_wasi_shim` in the worker.
