| Document | Size | stdlib tomllib | tomllib_rs | Speedup | Same result |
|---|---|---|---|---|---|
| lockfile, 3000 packages | 717 KB | 111.8 ms | 17.56 ms | 6.4× | yes |
| config, 2000 tables | 453 KB | 162.7 ms | 20.17 ms | 8.1× | yes |
| small pyproject-like | 1 KB | 0.4 ms | 0.06 ms | 7.1× | yes |
