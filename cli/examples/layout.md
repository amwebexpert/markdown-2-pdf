# Layout and pagination

Tables size their columns from their content, repeat their header on every
page, and never split a row unless it is taller than a page.

## Wide content

| Link | Notes |
| --- | --- |
| https://example.com/a/rather/long/path/to/some/resource | The URL column keeps its longest word whole; this prose column wraps instead of squeezing it into character fragments. |
| `src/features/Configuration/ReportTemplateEditor.tsx` | Long code spans behave the same way. |

## A long table

| # | Name | Notes |
| --: | --- | --- |
| 1 | Item number 1 | short |
| 2 | Item number 2 | short |
| 3 | Item number 3 | a longer note that has to wrap inside its column because it is quite wordy |
| 4 | Item number 4 | short |
| 5 | Item number 5 | short |
| 6 | Item number 6 | a longer note that has to wrap inside its column because it is quite wordy |
| 7 | Item number 7 | short |
| 8 | Item number 8 | short |
| 9 | Item number 9 | a longer note that has to wrap inside its column because it is quite wordy |
| 10 | Item number 10 | short |
| 11 | Item number 11 | short |
| 12 | Item number 12 | a longer note that has to wrap inside its column because it is quite wordy |
| 13 | Item number 13 | short |
| 14 | Item number 14 | short |
| 15 | Item number 15 | a longer note that has to wrap inside its column because it is quite wordy |
| 16 | Item number 16 | short |
| 17 | Item number 17 | short |
| 18 | Item number 18 | a longer note that has to wrap inside its column because it is quite wordy |
| 19 | Item number 19 | short |
| 20 | Item number 20 | short |
| 21 | Item number 21 | a longer note that has to wrap inside its column because it is quite wordy |
| 22 | Item number 22 | short |
| 23 | Item number 23 | short |
| 24 | Item number 24 | a longer note that has to wrap inside its column because it is quite wordy |
| 25 | Item number 25 | short |
| 26 | Item number 26 | short |
| 27 | Item number 27 | a longer note that has to wrap inside its column because it is quite wordy |
| 28 | Item number 28 | short |
| 29 | Item number 29 | short |
| 30 | Item number 30 | a longer note that has to wrap inside its column because it is quite wordy |
| 31 | Item number 31 | short |
| 32 | Item number 32 | short |
| 33 | Item number 33 | a longer note that has to wrap inside its column because it is quite wordy |
| 34 | Item number 34 | short |
| 35 | Item number 35 | short |
| 36 | Item number 36 | a longer note that has to wrap inside its column because it is quite wordy |
| 37 | Item number 37 | short |
| 38 | Item number 38 | short |
| 39 | Item number 39 | a longer note that has to wrap inside its column because it is quite wordy |
| 40 | Item number 40 | short |
| 41 | Item number 41 | short |
| 42 | Item number 42 | a longer note that has to wrap inside its column because it is quite wordy |
| 43 | Item number 43 | short |
| 44 | Item number 44 | short |
| 45 | Item number 45 | a longer note that has to wrap inside its column because it is quite wordy |
| 46 | Item number 46 | short |
| 47 | Item number 47 | short |
| 48 | Item number 48 | a longer note that has to wrap inside its column because it is quite wordy |
| 49 | Item number 49 | short |
| 50 | Item number 50 | short |
| 51 | Item number 51 | a longer note that has to wrap inside its column because it is quite wordy |
| 52 | Item number 52 | short |
| 53 | Item number 53 | short |
| 54 | Item number 54 | a longer note that has to wrap inside its column because it is quite wordy |
| 55 | Item number 55 | short |
| 56 | Item number 56 | short |
| 57 | Item number 57 | a longer note that has to wrap inside its column because it is quite wordy |
| 58 | Item number 58 | short |
| 59 | Item number 59 | short |
| 60 | Item number 60 | a longer note that has to wrap inside its column because it is quite wordy |

## A tall cell

| Key | Value |
| --- | --- |
| multi-line | line 1<br>line 2<br>line 3<br>line 4<br>line 5<br>line 6<br>line 7 |

## A long code block

```rust
fn main() {
    let value_1 = compute(1);
    let value_2 = compute(2);
    let value_3 = compute(3);
    let value_4 = compute(4);
    let value_5 = compute(5);
    let value_6 = compute(6);
    let value_7 = compute(7);
    let value_8 = compute(8);
    let value_9 = compute(9);
    let value_10 = compute(10);
    let value_11 = compute(11);
    let value_12 = compute(12);
    let value_13 = compute(13);
    let value_14 = compute(14);
    let value_15 = compute(15);
    let value_16 = compute(16);
    let value_17 = compute(17);
    let value_18 = compute(18);
    let value_19 = compute(19);
    let value_20 = compute(20);
    let value_21 = compute(21);
    let value_22 = compute(22);
    let value_23 = compute(23);
    let value_24 = compute(24);
    let value_25 = compute(25);
    let value_26 = compute(26);
    let value_27 = compute(27);
    let value_28 = compute(28);
    let value_29 = compute(29);
    let value_30 = compute(30);
    let value_31 = compute(31);
    let value_32 = compute(32);
    let value_33 = compute(33);
    let value_34 = compute(34);
    let value_35 = compute(35);
    let value_36 = compute(36);
    let value_37 = compute(37);
    let value_38 = compute(38);
    let value_39 = compute(39);
    let value_40 = compute(40);
    let value_41 = compute(41);
    let value_42 = compute(42);
    let value_43 = compute(43);
    let value_44 = compute(44);
    let value_45 = compute(45);
    let value_46 = compute(46);
    let value_47 = compute(47);
    let value_48 = compute(48);
    let value_49 = compute(49);
    let value_50 = compute(50);
    let value_51 = compute(51);
    let value_52 = compute(52);
    let value_53 = compute(53);
    let value_54 = compute(54);
    let value_55 = compute(55);
    let value_56 = compute(56);
    let value_57 = compute(57);
    let value_58 = compute(58);
    let value_59 = compute(59);
    let value_60 = compute(60);
}
```

> A blockquote keeps a bar on each page it spans, and headings are kept with
> the first lines of their section.
