# pdf2md — Phase 2 spec: images + tables

Status: ready · Depends on: phase 1 (`feat/pdf2md-phase-1`) · One PR.

## Context

Phase 1 rebuilds text, headings, lists, emphasis, links and code. Tables still
come out as flat paragraphs (`**Name Role Notes** Ada Engineer …`) and images are
dropped. Phase 2 closes both gaps. Decisions already settled while grilling the
plan:

| # | Decision |
|---|----------|
| Q6 | Images: assets folder next to the `.md` by default (`doc.md` + `doc_assets/img-001.png`, relative links); `--images=inline` writes base64 data URIs. WASM (phase 3) returns `{ markdown, assets }`. |
| Q7 | JPEG (`DCTDecode`) copied as is; Flate/LZW/raw bitmaps re-encoded as PNG (alpha from `SMask`); `JPXDecode` / `CCITTFaxDecode` / `JBIG2Decode` → `<!-- unsupported image (JPXDecode) -->`. Vector drawings are not rasterised. |
| Q8 | Tables: drawn rulings first, text-column alignment as fallback. Output GFM only (no HTML): multi-line cells joined with `<br>`, spanning cells flattened. |

## Goals

1. Raster images extracted, written as assets (or inlined) and referenced at
   their reading-order position.
2. Bordered tables (cell rectangles or line grids) and rule-only tables
   (booktabs: top/mid/bottom rules) rebuilt as GFM tables.
3. Borderless tables rebuilt from text alignment when the evidence is strong.
4. md2pdf tables round-trip: `cli/examples/kitchen-sink.md`'s table converts
   back to the same cells **and alignments**.
5. Bonus (cheap once rulings exist): a lone long horizontal rule → `---`.

## Non-goals

- Rasterising vector figures/charts; OCR of images; figure captions as alt text
  (alt stays empty).
- HTML tables, `colspan`/`rowspan`.
- Tables spanning pages being merged into one table (each page fragment becomes
  its own table; revisit later).

## Public API changes (`pdf2md-core`)

```rust
pub struct Options {
    pub pages: Option<(u32, u32)>,
    pub keep_headers: bool,
    pub images: ImageMode,            // new
}

#[derive(Debug, Clone)]
pub enum ImageMode {
    /// Assets referenced under `dir`, relative to the Markdown file. The caller
    /// picks the name (CLI: `<stem>_assets`); the core only uses it in links.
    /// `Default` = `Assets { dir: "images".into() }`.
    Assets { dir: String },
    /// Embed `data:image/...;base64,` URIs; `Output::assets` stays empty.
    Inline,
}

pub struct Output {
    pub markdown: String,
    pub page_count: usize,
    pub warnings: Vec<String>,
    pub assets: Vec<Asset>,           // new
}

pub struct Asset {
    pub path: String,       // e.g. "doc_assets/img-001.png", as written in the link
    pub mime: &'static str, // "image/png" | "image/jpeg"
    pub bytes: Vec<u8>,
}
```

The core stays I/O-free: the CLI writes `assets` to disk; WASM (phase 3) hands
them to JS.

## CLI (`cli/src/bin/pdf2md.rs`)

```text
pdf2md in.pdf [-o out.md] [--pages 2-5] [--keep-headers]
              [--images dir|inline]      # default dir
              [--assets-dir PATH]        # default <out stem>_assets, relative to out.md's folder
```

- Creates the assets directory only if there is at least one asset.
- Overwrites files with the same name in an existing assets dir.

## Design

### 1. Interpreter: collect rulings and image placements (`interp.rs`)

Today `path_op` keeps only a path bounding box (for vector bullets). Extend it to
keep **subpaths as segments** in user space:

```text
m x y            → start point
l x y            → segment(prev, p)
re x y w h       → 4 segments (closed rect) + remember the rect itself
c / v / y        → curve: bbox only (never a ruling)
h                → close segment
S s              → stroked: emit segments as rulings (with line width from `w`)
f F f* B B* b b* → filled: thin rect (w or h < 2pt) → ruling along its long axis;
                   small square → bullet (existing); anything else ignored
n                → discard
```

- Track `w` (line width) in `GraphicsState`, transformed by CTM scale.
- Keep only axis-aligned segments (|dx| or |dy| < 0.5pt) longer than 3pt.
- md2pdf draws each table cell as a stroked `re` (`core/src/render.rs`,
  `draw_table_row`): those give 4 rulings per cell — the grid builder must
  dedupe overlapping collinear rulings.
- Ignore white strokes/fills? (`RG`/`rg`/`g`/`G`/`k`/`K` colour tracking) — a
  white rule is invisible; track fill/stroke gray level and drop near-white.

`Do` on an `/Subtype /Image` XObject (and inline images: lopdf already parses
`BI … ID … EI` into a `"BI"` operation carrying an `Object::Stream`):

```rust
pub struct ImagePlacement {
    pub id: Option<ObjectId>,   // None for inline images (no dedupe)
    pub bbox: [f32; 4],         // unit square × CTM, user space
    pub stream: &'a Stream,     // or owned for inline images
}
```

- Skip images whose placed size is < 8pt in either dimension (spacers, rules
  drawn as 1px images) and `ImageMask true` stencils.
- `Page` gains `rulings: Vec<Ruling>` and `images: Vec<ImagePlacement>`.

### 2. Image decoding (`images.rs`, new)

```text
decode(stream) -> Result<EncodedImage { mime, bytes }, Unsupported(filter)>
  filters = /Filter chain
  last filter DCTDecode  → decode preceding filters (lopdf), keep JPEG bytes as is
                           (CMYK JPEG: keep, note in warnings — browsers may invert)
  JPX/CCITT/JBIG2        → Unsupported(name)
  otherwise              → pixels = lopdf decompressed_content_with_limit
                           to RGB(A) 8-bit:
                             BitsPerComponent 1/2/4/8/16 (16 → high byte)
                             ColorSpace DeviceGray | DeviceRGB | DeviceCMYK (→ RGB naive)
                                        | ICCBased (use /N: 1,3,4) | Indexed (palette lookup)
                                        | CalGray/CalRGB as Gray/RGB
                             /Decode [1 0] inversion
                             SMask (same w×h) → alpha; otherwise ignore SMask
                           → PNG via `png` crate (Gray, GrayAlpha, RGB, RGBA)
```

- Dedupe by `ObjectId`: an image drawn on every page (logo) yields one asset,
  linked each time. Naming: `img-001.png` in first-appearance order.
- Header/footer logic: an image that repeats in the furniture band on ≥ half the
  pages (same `ObjectId`) is dropped like repeated text, unless `--keep-headers`.
- Size guard: refuse images with `w*h > 50M` pixels (warning + placeholder).

New dependency: `png` (MIT OR Apache-2.0, pure Rust, WASM-friendly). Base64 for
`--images=inline` is hand-rolled (~25 lines, no dependency).

### 3. Layout: tables and images as page items (`layout.rs`)

Layout currently produces `Vec<Line>`. Generalise to reading-ordered **items**:

```rust
pub enum Item {
    Line(Line),
    Image { index: usize, bbox: [f32; 4], page: u32, column: Column, region: usize },
    Table(Table),
    Rule { page: u32, y: f32, column: Column, region: usize },
}
```

Order of operations per page:

```text
segments(page)                         # phase 1
rulings → grids                        # new, before gutter detection
glyph lines inside a grid → removed from segments, assigned to cells
alignment tables (fallback) on remaining segments
find_gutter(remaining segments + table/image boxes as obstacles)
column_groups / reading_order over Lines, Tables, Images, Rules (by top y)
```

Tables and images take part in column assignment by their bbox (x-extent vs
gutter) exactly like segments, so a figure inside the left column stays there.

#### 3a. Grid tables (rulings)

```text
H = horizontal rulings, V = vertical rulings (merged: collinear within 1pt,
    overlapping or touching within 2pt)
cluster connected rulings (a ruling touches another within 2pt) → candidate grids
grid needs ≥ 2 H and ≥ 2 V spanning it, or the booktabs shape (see 3b)
row boundaries = distinct H y's; column boundaries = distinct V x's
cell(r, c) = rectangle between consecutive boundaries
glyph → cell by glyph centre; cell text = phase-1 line/run logic on its glyphs
a missing internal boundary segment between two cells = spanning cell:
    content goes to the first cell, the others stay empty
```

#### 3b. Rule-only tables (booktabs, many Word/LaTeX styles)

Only horizontal rulings, ≥ 2 of them, of (nearly) equal x-extent, stacked with
text between: rows = text baselines between the first and last rule; columns
from 3c alignment restricted to that box.

#### 3c. Alignment tables (no rulings)

Candidate when ≥ 3 consecutive lines (same page/region, regular leading) each
split into ≥ 2 phase-1 segments (gap > `SEGMENT_GAP`), and the gaps line up:

```text
for each candidate run of lines:
  column gaps = x-intervals empty in every row (allow 1 row to miss for wrapped cells)
  need ≥ 1 shared gap, ≥ 2 columns, ≥ 3 rows, and ≥ 60% of cells non-empty
  reject if the run looks like a two-column page (≥ 2 columns each > 35% of
  the text width and lines ending justified) → leave to find_gutter
```

Conservative by design: a false table is worse than a missed one.

#### Cell alignment

Per column, compare each cell's text box to the column's inner box:
left gaps ≈ 0 → `:--`; equal left/right gaps → `:-:`; right gap ≈ 0 → `--:`.
Majority vote over body rows; no clear majority → `---`. This is what makes the
md2pdf kitchen-sink table (`:---`, `:--:`, `---:`) round-trip.

#### Header row

GFM requires one. Use the first row. (Bold-only first row is the common case
and is the header; nothing to detect.)

### 4. Blocks + emitter (`blocks.rs`, `emit.rs`)

New block variants:

```rust
Block::Image { src: String, alt: String }          // alt empty for now
Block::Table { align: Vec<Align>, rows: Vec<Vec<Vec<Run>>> }  // rows[0] = header
Block::Rule
```

- An image/table/rule item closes the open paragraph/list/code block.
- Every image is its own block, including small icons on a text line.
- Emit:
  - image: `![](doc_assets/img-001.png)` or `![](data:image/png;base64,…)`;
    unsupported: `<!-- unsupported image (JPXDecode) -->` + warning.
  - table: `| a | b |` / `| :-- | --: |` / rows. Cell text: inline renderer from
    phase 1, `|` escaped as `\|`, line breaks inside a cell → `<br>`, empty cell
    → single space. Pad columns? No (keep diffs small, like the source).
  - rule: `---`.

## Tests

Unit (in-module):

- `images.rs`: Gray/RGB/CMYK/Indexed conversion, 1/4/16-bit unpacking, `/Decode`
  inversion, SMask alpha, PNG signature + dimensions, JPEG passthrough bytes
  equal input, unsupported filter error, base64 known vectors.
- Grid builder: synthetic rulings → expected boundaries; md2pdf-style per-cell
  `re` rectangles deduped; spanning cell; booktabs three-rule table.
- Alignment tables: positive 3×3; negative two-column prose; negative bullet
  list with tab-like gaps.
- Cell alignment vote.
- Interp: `re`+`S` yields 4 rulings; thin filled rect yields 1; curve yields none;
  white stroke dropped; image `Do` placement bbox from CTM.

Integration:

- `roundtrip.md` gains the kitchen-sink style table (with alignments) and a
  `---` rule → still **exact** round trip.
- Kitchen-sink snapshot updated (table now a GFM table).
- New Chrome fixture `external/chrome-media.html/.pdf` (authored here, MIT):
  bordered table with a multi-line cell, borderless aligned table, a PNG with
  alpha, a JPEG, the same logo twice (dedupe). Assert asset count, mimes,
  PNG decodes (`png` crate in test), Markdown snapshot.
- arXiv fixture: add the page with **Table I** (REVTeX booktabs-style) of
  2610.40745 (CC BY 4.0, same provenance note) → snapshot asserts a GFM table.
- CLI: `--images inline` produces no assets dir; default writes
  `out_assets/img-001.*` and the links resolve (test with a temp dir).

## Acceptance criteria

- [ ] `cargo fmt --check`, clippy (native + `md2pdf-wasm` wasm32), tests,
      `cargo deny`, `cargo machete` green.
- [ ] Round trip exact for the extended `roundtrip.md` (table + rule).
- [ ] Chrome media fixture: 3 assets (logo deduped), tables as GFM.
- [ ] arXiv Table I page: a GFM table with the right column count.
- [ ] No regression in phase-1 snapshots other than intended table/image changes.
- [ ] README: pdf2md section updated (flags, how tables/images are recovered,
      limitations).
- [ ] 310-page text PDF still converts in well under 1s (release).

## Files

```diff
 pdf2md-core/src/
   interp.rs     # rulings, line width, colours, image placements, inline images
   page.rs       # Page { rulings, images }
+  images.rs     # filter chain, colour spaces, PNG/JPEG, base64
+  tables.rs     # grid + booktabs + alignment detection, cell alignment
   layout.rs     # Item enum, obstacles in gutter detection, item reading order
   blocks.rs     # Block::{Image, Table, Rule}
   emit.rs       # GFM table/image/rule output
   lib.rs        # ImageMode, Asset, Output::assets
 cli/src/bin/pdf2md.rs   # --images, --assets-dir, write assets
 pdf2md-core/Cargo.toml  # + png
 README.md, pdf2md-core/tests/fixtures/README.md
```

## Resolved questions (defaults adopted)

1. Alt text: empty in phase 2 (captions not used).
2. Small inline images (icons on a text line): emitted as their own block.
3. Existing non-empty assets dir: files with the same name are overwritten.
4. Tables split across pages: emitted as two tables (no merge).
5. CMYK JPEGs: kept as JPEG (passthrough) with a warning.
