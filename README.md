# 音楽絵 (Ongakue)

Music visualizer in Rust, running in the browser (WebAssembly + WebGPU/WebGL2) and natively.

Live version: https://antaalt.github.io/ongakue/ (deployed from `main` by GitHub Actions)

## Layout

- `crates/analysis`: FFT and frequency bands, pure Rust (`cargo test -p analysis`)
- `crates/render`: wgpu renderer, independent of windowing and audio
- `crates/app`: entry point, window and event loop (winit)

## Controls

- Pick an audio file, then press **Play**
- Click the visual or press **Space** to switch visuals

## Running

Web (requires [trunk](https://trunkrs.dev) and the `wasm32-unknown-unknown` target):

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
trunk serve --open
```

Native:

```sh
cargo run
```
