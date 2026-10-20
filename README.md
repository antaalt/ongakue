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
- The computer keyboard is a piano, with sound: the middle letter row plays
  white keys from middle C (Q S D F G H J K L M on AZERTY, A S D F… on QWERTY),
  the row above the black keys, and W / X (Z / X on QWERTY) change octave.
  Keys typed in a text field, like the shader editor, play nothing
- **Edit shader** opens the current visual's code in a code editor, with a picker
  at the top to switch visuals: it recompiles as you type, shows errors, and is
  saved in the browser, which also reopens the visual you had open last.
  With the [language server](#the-language-server), suggestions and
  explanations come from it, with signatures as you type a call, F12 to go to
  a definition, Shift+F12 to find references and F2 to rename; its errors show
  along the compile error. The **LSP** badge shows its state. With the **Auto** (compile) switch off, edits run only on
  **Compile** (▶, Ctrl+Enter). Loops stop after 100,000 iterations per pixel (all
  loops together), so an endless loop can't hang the GPU. A shader that still
  does (too much work per pixel) makes
  the browser reset it: the app recovers, runs a safe version of that visual
  until it's fixed and compiled, and turns auto compile off. It suggests names as you type (↑ ↓, Enter to accept),
  explains the name under the mouse, and has search (Ctrl+F) and undo.
  **New** (＋) creates a visual from a starter shader, **Import** (📂) opens a `.wgsl`
  file as a new visual, **Export** (💾, or Ctrl+S) downloads the current one,
  **Share** (🔗) copies a link carrying its code (opening the link adds it as a
  visual), and **Delete** (🗑) removes the ones you added. Visuals can be written in **WGSL** (`fs_main`) or
  in **GLSL**, Shadertoy style (`mainImage`, with `iTime` and `iResolution`):
  many Shadertoy shaders work as pasted (see the built-in "glsl" visual)
- **Stats** shows the frame rate, frame times (average and worst), the CPU
  time to analyze the sound and to submit each frame, the size drawn at, and
  the GPU and API in use (WebGPU or WebGL2). Natively, they're in the title
- **Tuning** opens sliders for the analysis settings and the shaders' free
  parameters (`u.params`), with a live plot of the beat detection. Each
  parameter's **MIDI** button links it to a knob: click it, then turn the knob

## Running

Web (requires [trunk](https://trunk-rs.github.io/trunk/) and the `wasm32-unknown-unknown` target):

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
trunk serve --open
```

Native:

```sh
cargo run
```

### The language server

The shader editor runs
[shader-language-server](https://github.com/antaalt/shader-sense) in the
browser: its WASI build, in workers (`web/lsp/`),
started the first time the editor opens. `web/codemirror/lsp.js` connects it to
the editor, showing it each visual's code with the code shared by all visuals
around it, like when compiling.

The server's wasm file (20 MB) isn't committed: the Pages workflow downloads
it. To run it locally, download it into `web/lsp/` (without it, the editor
uses its own suggestions):

```sh
curl -LO https://github.com/antaalt/shader-sense/releases/download/nightly/shader-language-server-wasm32-wasip1-threads.zip
unzip -o shader-language-server-wasm32-wasip1-threads.zip shader-language-server.wasm -d web/lsp
```

It needs shared memory, so a cross-origin isolated page: as GitHub Pages can't
set the headers for it, `web/coi-serviceworker.js`
([coi-serviceworker](https://github.com/gzuidhof/coi-serviceworker)) does,
reloading the page once on the first visit.

### The shader editor

The editor is [CodeMirror](https://codemirror.net/), bundled into
`web/codemirror/codemirror.js`, which is committed: building the app doesn't
need Node. To change the editor (`web/codemirror/editor.js`) or update
CodeMirror, rebuild the bundle with Node installed:

```sh
cd web/codemirror
npm install
npm run build
```
