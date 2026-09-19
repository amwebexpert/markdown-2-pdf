# md2pdf

Markdown → PDF, in Rust, sharable as a native CLI or a WASM module driven from
a Web Worker via OPFS.

**Repository:** [github.com/amwebexpert/markdown-2-pdf](https://github.com/amwebexpert/markdown-2-pdf)

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
  - [Just (task runner)](#just-task-runner)
  - [Build the CLI](#build-the-cli)
  - [Build the WASM module](#build-the-wasm-module)
  - [Bump versions](#bump-versions)
  - [Publish the WASM npm package](#publish-the-wasm-npm-package)
  - [Run the demo](#run-the-demo)
  - [Known limitations (v1)](#known-limitations-v1)

## Layout

- [`core/`](core/) — pure conversion logic (Markdown parsing via `pulldown-cmark`,
  hand-rolled text layout and PDF generation via `printpdf`). No I/O, no
  wasm-bindgen. Compiles to both native and `wasm32-unknown-unknown`
  unmodified.
- [`cli/`](cli/) — `md2pdf` binary (`clap`), reads a `.md` file, writes a `.pdf` file.
- [`wasm/`](wasm/) — `wasm-bindgen` wrapper exposing `convert(path)` to JS. This is
  the only crate that touches OPFS; it must run inside a Web Worker (OPFS's
  synchronous access handle is worker-only).
- [`demo/`](demo/) — minimal Vite + TypeScript page exercising the WASM module.

## Prerequisites

- Rust via `rustup` — [`rust-toolchain.toml`](rust-toolchain.toml) pins the exact version and adds
  the `wasm32-unknown-unknown` target automatically on first build.
- [`wasm-pack`](https://rustwasm.github.io/wasm-pack/) — `cargo install wasm-pack`.
- [`bun`](https://bun.sh) — for the demo page only.
- [`just`](https://github.com/casey/just) (optional) — `cargo install just` or `brew install just`; wraps the commands below.

## Just (task runner)

[`justfile`](justfile) at the repo root. Run `just` to list recipes.

| Recipe | Purpose |
| ------ | ------- |
| `build-cli` / `build-cli-release` | Build the `md2pdf` binary (debug / release). |
| `build-wasm` | `wasm-pack build wasm --target web` → `wasm/pkg/`. |
| `build-wasm-npm` | Same with `--scope amwebexpert` for npm publish. |
| `convert INPUT` | `cargo run -p md2pdf -- convert …` (optional extra flags after the path). |
| `examples` | Regenerate PDFs for every `cli/examples/*.md`. |
| `demo` | Build WASM, `bun install`, Vite dev server. |
| `demo-build` | WASM + production build of `demo/dist/`. |
| `check` / `clippy` / `fmt` / `fmt-check` | Workspace `cargo check`, clippy, format. |
| `publish-npm` | Scoped WASM build + `npm publish --access public` in `wasm/pkg/`. |
| `clean` | `cargo clean` and remove `wasm/pkg/`, demo `node_modules` / `dist/`. |

## Build the CLI

```sh
cargo build -p md2pdf
cargo run -p md2pdf -- convert cli/examples/kitchen-sink.md
# -o is optional; defaults to the input path with a .pdf extension
cargo run -p md2pdf -- convert cli/examples/kitchen-sink.md -o /tmp/out.pdf
```

See [`cli/examples/`](cli/examples/) for sample inputs (`simple.md`, `kitchen-sink.md` — every
supported construct, `nested-and-long.md` — nested lists and multi-page
pagination). Regenerate `.pdf` files with the `convert` commands above for manual comparison
(example PDFs are not committed; see [`.gitignore`](.gitignore)).

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
fonts (~2.7MB raw, unsubsetted — see [`core/src/fonts.rs`](core/src/fonts.rs)).

## Bump versions

All Rust crates share one semver via the workspace:

| Crate (directory) | Package name  | Version source                                                        |
| ----------------- | ------------- | --------------------------------------------------------------------- |
| [`core/`](core/)  | `md2pdf-core` | [`Cargo.toml`](Cargo.toml) → `[workspace.package] version`            |
| [`cli/`](cli/)    | `md2pdf`      | same                                                                  |
| [`wasm/`](wasm/)  | `md2pdf-wasm` | same (becomes npm `@amwebexpert/md2pdf-wasm` after `wasm-pack build`) |

Each member crate sets `version.workspace = true` in its own `Cargo.toml`; do **not**
duplicate a `version = "…"` line there unless you intentionally opt out of the workspace
(default is one bump for everyone).

**Release checklist:**

1. Edit `[workspace.package] version` in the repo-root [`Cargo.toml`](Cargo.toml)
   (e.g. `0.1.1` → `0.1.2`). Use [semver](https://semver.org/) — breaking API or output changes → major; backward-compatible features → minor; fixes → patch.
2. Rebuild what you ship: `cargo build -p md2pdf` for the CLI,
   `wasm-pack build wasm --target web` for the browser module (regenerates `wasm/pkg/package.json`
   with the new version).
3. Commit [`Cargo.toml`](Cargo.toml) and [`Cargo.lock`](Cargo.lock) together with the release tag or PR.

The [`demo/`](demo/) app is private (`demo/package.json` has its own `version`); bump it only if you care about local bookkeeping — it is not published.

## Publish the WASM npm package

`wasm-pack` writes an npm package under `wasm/pkg/` (gitignored). The Rust crate
is `md2pdf-wasm`; we publish to npm as **`@amwebexpert/md2pdf-wasm`** via
`--scope amwebexpert` (you must control that npm scope). [Bump versions](#bump-versions)
first, then rebuild and publish.

**Prerequisites:** an [npmjs.com](https://www.npmjs.com/) account and
[`npm login`](https://docs.npmjs.com/cli/v11/commands/npm-login).

[`wasm-pack build`](https://rustwasm.github.io/docs/wasm-pack/commands/build.html)
generates `package.json` from the wasm crate's `Cargo.toml` and copies
`LICENSE` and `README.md` into `wasm/pkg/`. npm renders that README on the
package page; `description`, `homepage`, `repository`, and `keywords` in
[`wasm/Cargo.toml`](wasm/Cargo.toml) fill the sidebar (edit [`wasm/README.md`](wasm/README.md) for npm-facing docs).

```sh
wasm-pack build wasm --target web --scope amwebexpert
cd wasm/pkg && npm publish --access public
```

Consumers install with `npm install @amwebexpert/md2pdf-wasm`, import `init` and `convert`,
and run the module in a **Web Worker** with **OPFS** (same constraints as the
demo — see [`demo/src/main.ts`](demo/src/main.ts) and [`demo/src/worker.ts`](demo/src/worker.ts)).

## Run the demo

Build the WASM module first ([Build the WASM module](#build-the-wasm-module)). From the repo root:

```sh
wasm-pack build wasm --target web
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
