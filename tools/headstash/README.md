# ZK-Headstash


```
After configure: 
num_advice=10,
num_fixed=14,
num_instance=1
Permutation columns: 15
Degree: 9
fixed_commitments: 33
selectors: 70
permutation.bytes_length: 484
```


## SIZE BENCHMARK

`frontend/bench-size.mjs` reports what the claim page costs on chain. It calls `e2e/render_frontend_prop.py` `bundle()` (gzip -9, mtime 0, 128 KiB chunks), so the gzip sizes and chunk counts are exactly what `chunk-upload` stores. Brotli (q11, node zlib) is shown for comparison only and is never uploaded.

```sh
cd tools/headstash/frontend
make bench-size PROVER_DIR=/tmp/hsfe/prover   # or: npm run bench:size -- --prover-dir /tmp/hsfe/prover
node bench-size.mjs --diff old-size-report.json --json
```

For each file it prints raw, gzip and brotli KiB plus the chunk count, then a total line. It writes `frontend/size-report.json` with the per-chunk byte sizes and sha256 values. When a report already exists, or one is passed with `--diff`, it also prints the gzip byte and chunk deltas against it.

## TODO

## FORMAL VERIFICATION

### Key Pairing (Foriegn Field Arithmetic (Chinese-Remainder-Theorem (CRT)))
