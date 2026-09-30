# 音楽絵 (Ongakue)

Music visualizer in Rust, running in the browser (WebAssembly + WebGPU/WebGL2) and natively.

Live version: https://antaalt.github.io/ongakue/ (deployed from `main` by GitHub Actions)

## Layout

- `crates/analysis`: FFT and frequency bands, pure Rust (`cargo test -p analysis`)
- `crates/render`: wgpu renderer, independent of windowing and audio
- `crates/app`: entry point, window and event loop (winit)

## Controls

- Choose the sound source: a **File** (pick one, then press **Play**) or the
  **Microphone** (or any audio input, e.g. an instrument through an audio interface)
- **MIDI** connects MIDI keyboards and controllers (Chrome, Edge, Firefox): held
  notes light their frequency in the spectrum, and shaders read them with
  `note(n)` and knobs with `cc(n)`
- **Edit shader** opens the current visual's WGSL code, with a picker at the top to
  switch visuals: it recompiles as you type, shows errors, and is saved in the
  browser. It suggests names as you type
  (↑ ↓, Enter or Tab to accept) and explains the name under the mouse
- **Tuning** opens sliders for the analysis settings and the shaders' free
  parameters (`u.params`), with a live plot of the beat detection. Each
  parameter's **MIDI** button links it to a knob: click it, then turn the knob

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
