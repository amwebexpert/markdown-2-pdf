# Release notes

This document exercises every construct that **pdf2md** should rebuild exactly from a PDF produced by *md2pdf*, including ***strong emphasis*** and `inline code`.

## Getting started

Install the tool, then read the [documentation](https://example.com/docs) before running your first conversion. Lines in this paragraph are long enough to wrap across several lines of the rendered page, which checks that wrapped lines merge back into one paragraph.

### Options

- `--pages` selects a range
- `--keep-headers` keeps page furniture
  - nested items are indented
  - and keep their own markers
- the last item has a [link](https://example.com/last)

#### Steps

1. Convert the file
2. Review the output
3. Commit the result

```
fn main() {
    let answer = 42;

    println!("{answer}");
}
```

##### Fine print

Escaped characters such as 2 \* 3 or x \_ y survive the trip.
