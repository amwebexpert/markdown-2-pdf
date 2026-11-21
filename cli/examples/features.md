# Markdown features

Inline styles combine freely: **bold**, *italic*, `code`, ~~struck~~,
~~**struck bold** and [a struck link](https://example.com)~~, and styles
glued inside one word like **un**_believ_`able`.

## Task list

- [x] Completed item
- [ ] Pending item
  - [x] Nested completed item
  - [ ] Nested pending item
- A plain bullet for comparison

## Image

Local images are resolved relative to this file; the title becomes a caption.

![A bar chart](images/diagram.png "Figure 1 — a generated diagram")

A missing image falls back to its alt text and a warning:

![a picture that is not there](images/missing.png)

## Footnotes

A statement with a footnote.[^source] Another one,[^detail] and the first
again.[^source]

[^source]: Supporting information, shown as an endnote.

[^detail]: Footnotes are numbered in the order they are first referenced.

## Raw HTML

A `<br>` tag<br>breaks the line, other tags like <kbd>this</kbd> stay
literal, and <!-- hidden --> comments are dropped.

<div class="note">
  Block HTML is shown verbatim, never interpreted.
</div>

Prices in dollars such as $5 or $10 stay as written, and so does math source
like $E = mc^2$.
