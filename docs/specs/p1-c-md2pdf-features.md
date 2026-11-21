# md2pdf — P1-C spec: Markdown feature coverage

Status: in progress · Source: `improvements.md` §6 · One PR (`feat/md2pdf-features`).

## Context

`md2pdf-core` only turns on `Options::ENABLE_TABLES`. Strikethrough, task
lists and footnotes print as raw syntax, images print as `[image: alt]`, and
raw HTML blocks are meant to be dropped. Two bugs were also found:

- An HTML block leaves its stack frame open, so **every block before it is
  lost** (`<p>a</p>` after a heading drops the heading).
- Text is split into words per span, so `foo**bar**` renders as `foo bar` and
  `[x](u).` as `x .`: adjacent styles inside one word get a fake space.

## Compatibility matrix

"Event" = pulldown-cmark 0.13 already emits it given the option; no second
parser is needed for any row.

| Construct | Parser option / event | Before | After |
|---|---|---|---|
| Headings, paragraphs, emphasis, strong, code span | core | ✅ | ✅ |
| Adjacent styles inside a word (`foo**bar**`) | core | ❌ fake space | ✅ |
| Links (annotation) | core | ✅ | ✅ |
| Lists, nested lists | core | ✅ | ✅ |
| Code blocks (no highlighting) | core | ✅ | ✅ |
| Blockquotes, thematic breaks, hard breaks | core | ✅ | ✅ |
| GFM tables | `ENABLE_TABLES` | ✅ | ✅ |
| Strikethrough `~~x~~` | `ENABLE_STRIKETHROUGH` → `Tag::Strikethrough` | literal | ✅ drawn line |
| Task lists `- [x]` | `ENABLE_TASKLISTS` → `Event::TaskListMarker` | literal | ✅ vector checkbox |
| Footnotes `[^a]` | `ENABLE_FOOTNOTES` → `FootnoteReference` / `Tag::FootnoteDefinition` | literal | ✅ endnotes |
| Images (own paragraph) | core `Tag::Image` | `[image: alt]` | ✅ embedded PNG/JPEG |
| Images (inside running text) | core | `[image: alt]` | `[image: alt]` (documented) |
| Raw HTML block | core `Event::Html` | dropped (+ bug) | literal monospace text |
| Inline HTML | core `Event::InlineHtml` | literal | literal; `<br>` → line break |
| HTML comments | core | dropped/literal | dropped (author intent: hidden) |
| Math `$x$`, `$$x$$` | `ENABLE_MATH` | literal | literal (option stays off) |
| Definition lists | `ENABLE_DEFINITION_LIST` | literal | literal (deferred) |
| Super/subscript | `ENABLE_SUPERSCRIPT/SUBSCRIPT` | literal | literal (deferred) |
| Wiki links | `ENABLE_WIKILINKS` | literal | literal (deferred) |

## Design

### Strikethrough
`Span.strike: bool`. Drawn as a 0.6 pt line through the word at
`baseline + 0.3·size`, in the text color; the gap between two struck words
on the same line is struck too. Composes with bold/italic/code/link because
it is an independent flag.

### Adjacent styles
A `Word` becomes a list of styled fragments. Tokenization breaks only on real
whitespace or `\n`; a style change without whitespace extends the current
word. Width = sum of fragment widths. Character-level hard wrapping of an
oversized word works per fragment.

### Task lists
List items carry `task: Option<bool>` (from `Event::TaskListMarker`, which
pulldown-cmark emits first in the item). The marker slot draws a stroked
8 pt square; checked items add a two-segment check mark. Vector only — no
font glyph dependency. Text alignment is identical to bulleted items.

### Footnotes (endnote-style)
- `FootnoteReference(label)` → a span with `footnote: Some(n)`, rendered as
  `n` at 70 % size raised by 0.35·size. Numbers follow first-reference order.
- `FootnoteDefinition(label)` blocks are collected out of the flow, wherever
  they appear (before or after the reference).
- After the last block: a thematic break, then each definition prefixed with
  `n.` in reference order; definitions never referenced follow, numbered on.
- Limitation: notes are endnotes, not bottom-of-page footnotes. Long notes
  paginate like ordinary paragraphs.

### Images
- **Pure core.** New API, `convert` unchanged:

  ```rust
  pub struct Options { pub images: BTreeMap<String, Vec<u8>> } // key = URL as written
  pub struct Output { pub pdf: Vec<u8>, pub page_count: usize, pub warnings: Vec<String> }
  pub fn convert_with_options(markdown: &str, options: &Options) -> Result<Output, Md2PdfError>;
  pub fn image_sources(markdown: &str) -> Vec<String>;
  ```
  The core never reads files or fetches URLs.
- **CLI policy.** Relative paths resolve against the Markdown file's
  directory; absolute paths are used as is; URLs with a scheme (`http:`,
  `https:`, `data:`, …) are never fetched (warning). Unreadable files →
  warning. Warnings go to stderr; the conversion still succeeds.
- **WASM.** `markdownToPdf(markdown, images?)`: optional `Map<string,
  Uint8Array>` (or plain object). Old one-argument calls keep working.
- **Decoding** (no `image` crate): PNG via the `png` crate already in the
  workspace (palette/16-bit expanded to 8-bit gray/RGB, alpha dropped onto
  white); JPEG passed through as a `/DCTDecode` XObject (size and components
  read from the SOF marker). Anything else → fallback.
- **Layout.** Natural size = px × 0.75 pt (96 dpi). Scaled down (never up)
  to fit the content width and the printable height, aspect ratio kept. If
  it doesn't fit under the cursor and the cursor isn't at the top of a page,
  start a new page. A title (`"…"`) becomes a 9 pt italic caption under the
  image, kept on the same page.
- **Block vs inline.** A paragraph whose content is only images (and
  whitespace) becomes image blocks. Images mixed with text keep the
  `[image: alt]` text fallback.
- **Fallback.** Missing/unsupported image → italic `[image: alt]` paragraph
  + a warning naming the URL. Deterministic, never fails the conversion.

### Raw HTML
- Block HTML: rendered as literal source in the code-block style.
- Inline HTML: literal text, except `<br>`, `<br/>`, `<br />` → line break.
- `<!-- comments -->` (block or inline) are dropped.
- Nothing is interpreted: no scripts, no external resources.

### Math
`ENABLE_MATH` stays off: `$E = mc^2$` prints verbatim, so currency text is
never misread and the source is preserved. A future renderer can turn the
option on and map `InlineMath`/`DisplayMath` without parser changes.

## Tests

- Parser (IR) tests per construct in `core/src/markdown/mod.rs`, including
  the HTML-block regression, nested/adjacent styles, nested task lists,
  footnote definition before reference, unreferenced definition, currency.
- Render tests in `core/src/render.rs` asserting on emitted ops (strike
  line present, checkbox rectangle, image XObject, fallback text).
- Image decoding unit tests (PNG gray/RGB/RGBA/palette, JPEG SOF, garbage).
- Round trip `pdf2md-core/tests/roundtrip.rs`: strikethrough text, task text,
  footnote text, and an embedded PNG coming back as an image asset.
- CLI: relative image path resolved, missing image warns.

## Acceptance

- Every new construct has parser and render tests.
- Content before an HTML block is no longer lost.
- Images keep aspect ratio; fallback deterministic; WASM API has no fs.
- Footnote definitions are never lost.
- `just lint` and `just build-wasm` pass.
