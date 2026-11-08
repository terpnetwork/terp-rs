# Headstash claim page: tests and size benchmark (on-chain chunk sizes)
PROVER_DIR ?= /tmp/hsfe/prover
.PHONY: test bench-size
test:
	node --test test/*.mjs
bench-size:
	node bench-size.mjs --prover-dir $(PROVER_DIR)
