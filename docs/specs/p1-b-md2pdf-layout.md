# md2pdf — P1-B spec: layout and pagination

Status: ready · Source: `improvements.md` §5 · Depends on: P1-C (fragment
words) · One PR.

## Context (current renderer, `core/src/render.rs`)

- One top-down cursor; blocks are wrapped then drawn immediately. Only
  tables measure all rows first. `ensure_space` is called once per line.
- Text width uses real hmtx advances (`fonts::text_width_pt`); no kerning,
  no shaping; missing glyphs map to `.notdef`.
- **Tables:** column width = widest cell measured on one line in a single
  font picked by "any span bold/italic/code" (`measure_row_natural`); if the
  total overflows, every column shrinks by the same factor. No per-column
  minimum, so a column of long words can be squeezed below its longest word
  and get char-split. Header is never repeated; a row taller than the page
  runs off the bottom.
- **Blocks:** headings can be stranded at a page bottom; spacing before
  headings / after paragraphs is added without a space check (can push a
  line past the margin, or leave a gap at a page top); code blocks and
  blockquotes have no visual container.

## Goals

1. **Column sizing.** Per column: `min` = widest unbreakable word (span-aware
   fonts), `max` = natural one-line width (span-aware). If `Σmax ≤ avail` use
   `max`. Else if `Σmin ≤ avail`: start from `min`, distribute the slack in
   proportion to `max − min` (demand). Else: proportional to `min` (words
   will char-wrap — the only overflow policy; never shrink the font).
   Invariant tested: `Σwidths ≤ avail`.
2. **Cell measurement** uses the same `tokenize`/`wrap_lines` as
   paragraphs (already true); measurement fixed to be per span.
3. **Repeated header.** When a body row starts a new page, redraw the header
   row first. If header + first row don't fit after the heading, move the
   whole start of the table (keep-with-next).
4. **Tall rows.** A row taller than a full page is split at line
   boundaries: each part draws its own borders; documented fallback. No
   infinite loop: a fresh page always accepts at least one line.
5. **Keep-with-next.** A heading reserves its own height + 2 body lines
   (or the first table row / image) before drawing; else new page.
6. **Spacing hygiene.** Spacing before a block is dropped at the top of a
   page; spacing after never forces a page break; `finish` doesn't emit an
   empty trailing page.
7. **Code blocks / blockquotes.** Light grey background per page fragment
   for code; a left bar for blockquotes — drawn per page fragment so
   pagination stays line-by-line (no keep-together for long blocks).
8. **Orphans/widows.** Paragraphs of ≥ 4 lines keep at least 2 lines on each
   side of a break.

## Non-goals

Justification, hyphenation, kerning/shaping, font fallback for CJK
(documented as unsupported), a generic layout framework.

## Design notes

- Introduce a small measured `LineBox { height, draw: DrawFn-ish data }`
  only for paragraphs/tables where look-ahead is needed — no general box
  tree. Drawing consumes the measured lines (no re-measure).
- `ensure_space` gains a `keep: f32` variant used by headings.

## Tests

- Unit: column-width algorithm (fits, demand split, overflow → min-proportional,
  sum ≤ available), tall-row split, keep-with-next decisions.
- Render: a 120-row table → N pages, header text present on every page
  (via `pdf2md-core` round trip or op inspection), no empty page.
- Narrow content (deep list nesting) + long URL in a cell.
- Visual: render examples to PNG and inspect; review every snapshot diff.

## Acceptance

Tables fit the printable width; long cells wrap; row heights cover content;
headers repeat; no blank pages or loops; headings not stranded; examples
visually stable except intended changes; native + WASM builds pass.
