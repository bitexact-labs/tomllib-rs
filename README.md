# tomllib-rs: Python's `tomllib` with a Rust parser, 6–8× faster, identical behaviour

A drop-in replacement for the standard library TOML parser (`tomllib`, Python 3.11+; it is tomli inside).
Not "a TOML parser in Rust": a line-by-line port of tomllib's own parser, so the **values, the exception class,
the error messages and the error positions are the same**.

```python
import tomllib_rs as tomllib          # instead of: import tomllib
with open("pyproject.toml", "rb") as f:
    data = tomllib.load(f)
# errors are tomllib.TOMLDecodeError, same text: "Cannot declare ('a',) twice (at line 2, column 3)"
```

## Benchmark (Python 3.14, Rust 1.93, Lenovo M720q)
| Document | Size | stdlib tomllib | tomllib_rs | Speedup | Same result |
|---|---|---|---|---|---|
| lockfile, 3000 `[[package]]` tables | 717 KB | 111.8 ms | 17.6 ms | 6.4× | yes |
| config, 2000 tables with inline tables, dates, multiline strings | 453 KB | 162.7 ms | 20.2 ms | 8.1× | yes |
| small pyproject-like file | 1 KB | 0.40 ms | 0.06 ms | 7.1× | yes |
Reproduce: `python bench/bench.py`. Part of the remaining time is building Python objects (dicts, datetimes), which any parser has to do.

## Verification
- **The official [toml-test](https://github.com/toml-lang/toml-test) suite, all 776 valid and invalid files:** identical result for every file
  (same value, or the same exception with the same message, `pos`, `lineno`, `colno`).
- **Fuzzing:** 120,000 random mutations of those files (inserted/deleted/replaced brackets, quotes, escapes, control characters,
  CRLF, emoji, `"""`, `'''`): zero differences. Script: `tests/compare.py`.

## Scope and limits
- `loads`, `load`, `TOMLDecodeError` (re-exported from `tomllib`, so `except tomllib.TOMLDecodeError` keeps working).
- A custom `parse_float=` is passed to the standard library, so its exact call order is kept (no speedup in that case).
- Error messages quote keys with `repr()`. For keys containing rare invisible Unicode characters the `repr` approximation
  in Rust may differ from CPython's `str.isprintable()`; values are never affected.

## Build
```
pip install maturin
cd python && maturin develop --release
```

## License
MIT (same as tomli, which this is ported from). See `LICENSE`.
