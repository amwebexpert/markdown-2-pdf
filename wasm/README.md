# @amwebexpert/md2pdf-wasm

Markdown → PDF in the browser, compiled from Rust to WebAssembly.

**Source code, CLI, demo, and full documentation:**
[github.com/amwebexpert/markdown-2-pdf](https://github.com/amwebexpert/markdown-2-pdf)

## Install

```sh
npm install @amwebexpert/md2pdf-wasm
```

## Usage

### Markdown string → PDF bytes (any context)

Works on the main thread or in a Web Worker. Conversion is synchronous, so
move it to a worker for very large documents.

```ts
import init, { markdownToPdf } from "@amwebexpert/md2pdf-wasm";

await init(); // once

const pdfBytes = markdownToPdf("# Hello\n\nSome **bold** text.");
const blob = new Blob([new Uint8Array(pdfBytes)], { type: "application/pdf" });

const link = document.createElement("a");
link.href = URL.createObjectURL(blob);
link.download = "hello.pdf";
link.click();
```

### OPFS file → OPFS file (Web Worker only)

Reads a Markdown file from the OPFS (Origin Private File System) root, writes
the PDF back to OPFS, and returns the output filename. No large buffer crosses
the JS/WASM boundary. Must run in a **Web Worker** (OPFS's synchronous access
handle is worker-only).

```ts
import init, { convert } from "@amwebexpert/md2pdf-wasm";

await init();
const outputPath = await convert("input.md"); // "input.pdf"
```

See the [demo app](https://github.com/amwebexpert/markdown-2-pdf/tree/master/demo)
([`main.ts`](https://github.com/amwebexpert/markdown-2-pdf/blob/master/demo/src/main.ts),
[`worker.ts`](https://github.com/amwebexpert/markdown-2-pdf/blob/master/demo/src/worker.ts))
for a working Vite integration of both modes.

## API

- `init(input?)` — load the WASM module; call once per JS context (main thread
  or worker). By default it fetches `md2pdf_wasm_bg.wasm` next to the JS file;
  pass `{ module_or_path }` (URL, `Response`, bytes, or `WebAssembly.Module`) to
  load it from elsewhere.
- `markdownToPdf(markdown: string): Uint8Array` — PDF bytes. Throws on failure.
- `convert(inputPath: string): Promise<string>` — flat filename at OPFS root;
  resolves to the `.pdf` output filename. Web Worker only.

## Bundlers

The package is built with `wasm-pack --target web` (plain ES modules).

- **Vite, Rollup, esbuild, Parcel**: works as-is; `init()` resolves the `.wasm`
  through `new URL(..., import.meta.url)`.
- **webpack 5**: works as-is with its default `new URL(..., import.meta.url)`
  asset handling; if your config disables it, import the `.wasm` as an asset
  URL and pass it to `init({ module_or_path: url })`.
- **No bundler / CDN**: import the module file directly from a CDN that serves
  the package files as-is, so the `.wasm` resolves next to it, e.g.
  `import init, { markdownToPdf } from "https://cdn.jsdelivr.net/npm/@amwebexpert/md2pdf-wasm/md2pdf_wasm.js";`.

The `.wasm` is ~6 MB (~3 MB gzipped; fonts are embedded), so call `init()` lazily,
when the user first needs a PDF.

Other `Pdf_*` exports in the type definitions come from the bundled `printpdf`
dependency and are not part of this package's API.

## License

MIT — see [LICENSE](https://github.com/amwebexpert/markdown-2-pdf/blob/master/LICENSE).
