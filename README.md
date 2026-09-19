# md2pdf

Markdown → PDF, in Rust, sharable as a native CLI or a WASM module driven from
a Web Worker via OPFS.

[![standard-readme compliant](https://img.shields.io/badge/readme%20style-standard-brightgreen.svg?style=flat-square)](https://github.com/RichardLitt/standard-readme)
[![Rust](https://img.shields.io/badge/Rust-1.98.1-orange.svg?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![WebAssembly](https://img.shields.io/badge/WebAssembly-654FF0.svg?style=flat-square&logo=webassembly&logoColor=white)](https://webassembly.org/)
[![TypeScript](https://img.shields.io/badge/TypeScript-7.0-blue.svg?style=flat-square&logo=typescript)](https://www.typescriptlang.org/)
[![Vite](https://img.shields.io/badge/Vite-8-646CFF.svg?style=flat-square&logo=vite&logoColor=white)](https://vite.dev/)
[![Bun](https://img.shields.io/badge/Bun-000000.svg?style=flat-square&logo=bun)](https://bun.sh/)
[![wasm-pack](https://img.shields.io/badge/wasm--pack-Rust%20Wasm-654FF0.svg?style=flat-square&logo=webassembly&logoColor=white)](https://rustwasm.github.io/wasm-pack/)

- [md2pdf](#md2pdf)
  - [Layout](#layout)
  - [Prerequisites](#prerequisites)
  - [Build the CLI](#build-the-cli)
  - [Build the WASM module](#build-the-wasm-module)
  - [Run the demo](#run-the-demo)
  - [Known limitations (v1)](#known-limitations-v1)

## Layout

- `core/` — pure conversion logic (Markdown parsing via `pulldown-cmark`,
  hand-rolled text layout and PDF generation via `printpdf`). No I/O, no
  wasm-bindgen. Compiles to both native and `wasm32-unknown-unknown`
  unmodified.
- `cli/` — `md2pdf` binary (`clap`), reads a `.md` file, writes a `.pdf` file.
- `wasm/` — `wasm-bindgen` wrapper exposing `convert(path)` to JS. This is
  the only crate that touches OPFS; it must run inside a Web Worker (OPFS's
  synchronous access handle is worker-only).
- `demo/` — minimal Vite + TypeScript page exercising the WASM module.

## Prerequisites

- Rust via `rustup` — `rust-toolchain.toml` pins the exact version and adds
  the `wasm32-unknown-unknown` target automatically on first build.
- [`wasm-pack`](https://rustwasm.github.io/wasm-pack/) — `cargo install wasm-pack`.
- [`bun`](https://bun.sh) — for the demo page only.

## Build the CLI

```sh
cargo build -p md2pdf
cargo run -p md2pdf -- convert cli/examples/kitchen-sink.md
# -o is optional; defaults to the input path with a .pdf extension
cargo run -p md2pdf -- convert cli/examples/kitchen-sink.md -o /tmp/out.pdf
```

See `cli/examples/` for sample inputs (`simple.md`, `kitchen-sink.md` — every
supported construct, `nested-and-long.md` — nested lists and multi-page
pagination) with their committed `.pdf` output for manual comparison.

## Build the WASM module

```sh
wasm-pack build wasm --target web
```

Produces `wasm/pkg/` (gitignored): `md2pdf_wasm.js`, `md2pdf_wasm_bg.wasm`,
and a `.d.ts`. The demo imports this directly.

**Note:** `printpdf` (a dependency of `core`) bundles its own `wasm-bindgen`
API unconditionally for any wasm32 build (`Pdf_HtmlToDocumentSync` and
friends). These extra exports show up in `md2pdf_wasm.d.ts` alongside our own
`convert`/`init` — harmless, but not part of this project's API; ignore them.
They also add to the ~6MB binary size, on top of the embedded Liberation
fonts (~2.9MB raw, unsubsetted — see `core/src/fonts.rs`).

## Run the demo

```sh
cd demo
bun install
bun start
```

Open the printed local URL. Edit the Markdown, click **Convert to PDF** — it
writes the text to OPFS, runs the WASM module in a Worker (sync OPFS access
handle), reads the resulting PDF back from OPFS, and offers it as a download.
No large buffer is copied across the JS/WASM boundary directly.

## Known limitations (v1)

- Markdown support: headings, paragraphs, bold/italic, ordered/unordered
  lists (incl. nested), code blocks (no syntax highlighting), links
  (rendered as text + a real clickable PDF annotation), GFM tables
  (content-sized columns, alignment), blockquotes (indented), and thematic
  breaks (horizontal rules). Anything else CommonMark/GFM can emit (images,
  strikethrough, task lists, …) falls back to plain text rather than failing
  the conversion.
- No automated tests — verify manually via the CLI examples above or the demo.
- Fonts are embedded unsubsetted; OPFS paths are flat filenames (no
  subdirectories).
