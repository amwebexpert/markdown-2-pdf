# pdf2md — P1-A spec: table reconstruction

Status: implemented (partly, see below) · Source: `improvements.md` §4 · One
PR (`feat/pdf2md-tables`).

## As built

- **Goal 1:** `tables::Table` gained `columns` (boundaries), `spans`
  (`Span { row, col, rows, cols }`) and `evidence` (`Ruled | RuleOnly |
  Aligned`). Not added: per-cell bboxes, `header_rows` (row 0 is still the
  header), a numeric score.
- **Goal 2:** collinear/touch tolerances scale with the median glyph size
  (0.2/0.4 em, never below 1/2 pt). Partial verticals were already handled
  by the span union. Decorative-line filtering is not done.
- **Goal 3:** tightly set booktabs columns are split at the header words. The
  arXiv Table II snapshot is fixed (4 columns). "Header above the top rule"
  is not done, because no fixture shows it.
- **Goal 4:** not a weighted score; there is no fixture corpus to tune one.
  Instead `tables::verdict` names its rejections: numbered equations are
  rejected, and sparse near misses are reported as warnings.
- **Goal 5:** merged cells are recorded and a warning is emitted. The output
  is unchanged (text in the first cell).
- **Goal 6:** `continuation.rs` merges tables that span pages, as specified,
  with one warning per merge. Repeated headers are always dropped.
- **Goal 7:** warnings cover merged cells, near misses and cross-page merges.

## Context (current pipeline, `pdf2md-core/src/`)

- `interp.rs` collects `Ruling`s (stroked segments, thin filled rects).
- `tables.rs::ruled` merges collinear rulings (fixed `COLLINEAR` 1.0 pt,
  `TOUCH` 2.0), clusters by touch (union-find), builds a grid from **all**
  distinct x/y positions of the cluster, unions cells whose shared boundary
  has no ruling (spans), assigns glyphs by centre point, rejects < 30 %
  filled. Rule-only clusters → booktabs path (`aligned`, no row merge).
- Borderless: `layout.rs::alignment_tables` → `tables::aligned` (sweep for
  shared x-gaps, ≥ 3 rows, ≥ 60 % fill, rejects lists/monospace/2-col prose).
- Model: `Table { bbox, align, rows: Vec<Vec<Cell>> }`, row 0 = header.
  Spans are flattened (text in anchor, others empty). No warnings.
- Cells keep bold/italic/mono/links (`blocks::cell_runs` → `emit::inline`).
- Known misses: `external__arxiv_booktabs_table.snap` second table collapses
  to 2 columns; header above the top booktabs rule is outside the bbox;
  tables split across pages stay separate.

## Goals (in order)

1. **Explicit model** — extend `tables::Table` (no parallel model):
   `page`, `cells` with `bbox`, `row_span`, `col_span`; `header_rows: usize`;
   `evidence: Evidence { Ruled | RuleOnly | Aligned }` + `score: f32`.
   Emit code reads only this.
2. **Ruled tables.** Tolerances relative to median glyph size near the
   cluster (`COLLINEAR = 0.15·size`, `TOUCH = 0.25·size`, floor = current
   values). Partial rules: a vertical that covers < 50 % of the table
   height only splits the rows it crosses (span instead of a new global
   column). Decorative lines: drop rulings that cross no text row and lie
   outside the grid bbox. Vertically centered text: cell assignment already
   by centre — add test.
3. **Booktabs.** Extend the bbox up by one text line when the line directly
   above the top rule aligns with the detected gutters (header above rule).
   Fix the arXiv second-table collapse (gutter sweep must tolerate one
   spanning group-label row).
4. **Borderless scoring.** Score = weighted sum of: shared-gap count stable
   across rows, column start x-cluster tightness, baseline spacing
   regularity (stdev / median < 0.25), absence of sentence-like lines
   (no line ending in `.` that wraps into the next cell), header alignment.
   Threshold tuned on fixtures; below → paragraphs. Keep existing hard
   rejections.
5. **Merged cells → GFM fallback.** GFM can't span: anchor keeps the text,
   covered cells stay empty (current), plus a warning
   `page N: table has merged cells (flattened)`. Never drop text.
6. **Cross-page continuation** (separate pass over all pages' items):
   merge table B into A when A is the last item of page p (bottom within
   20 % of page), B the first non-furniture item of page p+1, same column
   count, every column boundary within `max(3 pt, 2 %)` of width. If B's
   row 0 equals A's header (normalized text) drop it. Low confidence →
   leave separate. `Options.keep_repeated_headers` not added unless asked.
7. **Diagnostics.** Warnings for: merged cells flattened, candidates
   rejected by score (only when score is within 20 % of threshold —
   "ambiguous"), tables merged across pages.

## Non-goals

HTML table output, OCR, ML, caption detection.

## Fixtures / tests

- Hand-built PDFs with lopdf (like `page.rs` tests): broken rules (gaps of
  1–3 pt), misaligned verticals (±1.5 pt), partial verticals, header above
  booktabs rule, empty cells, vertically centered cells, decorative rule
  beside a paragraph.
- md2pdf-generated: multi-page table (after P1-B repeats headers → tests
  header de-dup), cells with bold/italic/code/link.
- False positives: aligned prose, two-column prose, aligned equations
  (monospace), numbered list.
- Existing fixtures/snapshots unchanged except intended diffs (each
  explained in the PR).

## Acceptance

As `improvements.md` §4: existing fixtures green, imperfect rules recovered,
booktabs kept, false positives covered, merged text never lost, cross-page
merge only with strong evidence, native + WASM builds pass.
