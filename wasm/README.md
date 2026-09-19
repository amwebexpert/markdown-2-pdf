# @amwebexpert/md2pdf-wasm

Markdown → PDF in the browser, compiled from Rust to WebAssembly.

**Source code, CLI, demo, and full documentation:**
[github.com/amwebexpert/markdown-2-pdf](https://github.com/amwebexpert/markdown-2-pdf)

## Install

```sh
npm install @amwebexpert/md2pdf-wasm
```

## Usage

This module must run in a **Web Worker** with **OPFS** (Origin Private File System).
It reads a Markdown file from the OPFS root, writes the PDF back to OPFS, and returns
the output filename. See the [demo app](https://github.com/amwebexpert/markdown-2-pdf/tree/master/demo)
and [`demo/src/main.ts`](https://github.com/amwebexpert/markdown-2-pdf/blob/master/demo/src/main.ts)
for a working Vite integration.

```ts
import init, { convert } from "@amwebexpert/md2pdf-wasm";

await init();
const outputPath = await convert("input.md");
```

## API

- `init()` — load the WASM module (call once in the worker).
- `convert(inputPath)` — flat filename at OPFS root; returns the `.pdf` output path.

## License

MIT — see [LICENSE](https://github.com/amwebexpert/markdown-2-pdf/blob/master/LICENSE).
