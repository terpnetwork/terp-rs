#!/bin/sh
# 1. define terp account export prior to 2026-03-02
# 2. query api for `/cosmos/staking/v1beta1/validators?pagination.limit=300`
# 3. use `scripts/src/bin/parse_export.rs` to format csv 
# 4. use `scripts/src/bin/gen_merkle.rs` to generate merkle tree