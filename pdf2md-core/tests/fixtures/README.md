# pdf2md test fixtures

| File | Producer | Licence |
| --- | --- | --- |
| `roundtrip.md` | Written for this repo; rendered to PDF by md2pdf at test time. | MIT (this repo) |
| `external/chrome-handbook.pdf` | Google Chrome headless `--print-to-pdf` from `external/chrome-handbook.html` (written for this repo): vector list bullets, subset fonts with real names, CSS two-column section. | MIT (this repo) |
| `external/chrome-media.pdf` | Google Chrome headless `--print-to-pdf` from `external/chrome-media.html` (written for this repo, images generated for it): bordered table with a multi-line cell, borderless aligned table, a PNG with alpha, a JPEG, the same logo drawn twice. | MIT (this repo) |
| `external/arxiv-2610.40745-p1-3.pdf` | Pages 1–3 of Simon J. Porter, *What It Costs to Trust a Source: A Reproducible Method for Institutional Assessment of Author Disambiguation*, [arXiv:2610.40745](https://arxiv.org/abs/2610.40745) (REVTeX, two columns with a full-width abstract). Trimmed with lopdf (`delete_pages`, `prune_objects`); content unchanged. | [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) |
| `external/arxiv-2610.40745-p9.pdf` | Page 9 of the same paper (Table I, REVTeX rules without vertical lines), trimmed the same way. | [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) |

Regenerate the Chrome fixtures with:

```sh
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new \
  --print-to-pdf=external/chrome-handbook.pdf "file://$PWD/external/chrome-handbook.html"
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --no-pdf-header-footer \
  --print-to-pdf=external/chrome-media.pdf "file://$PWD/external/chrome-media.html"
```
