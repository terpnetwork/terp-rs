# js-deps: front-end JS dependency scraper

`gen-js-deps.mjs` reads each Terp front end's `package.json` and lockfile
(`pnpm-lock.yaml` v9 or `package-lock.json` v2/v3) and merges their full
dependency trees into one JSON graph. Nothing is installed or fetched. It
covers `dependencies` and `optionalDependencies` (direct and transitive) plus
the `devDependencies` of workspace members; a package reachable only through a
dev dependency is marked dev-only. Depth is the shortest path from a workspace
member (members are depth 0). Node ≥ 18 is the only requirement.

```sh
just gen-js-deps                    # from tools/gen-tools → ../../target/gen-deps/js-deps.json
just gen-js-deps path/to/out.json   # custom output
node js-deps/gen-js-deps.mjs --out out.json --only terp-network,permissionless --no-dev
```

Options: `--out FILE` (or a bare first argument), `--config FILE` (default
`roots.json` here), `--root id=PATH` (override or add a root), `--only a,b`,
`--no-dev`.

## Roots

`roots.json` lists the front ends under `terp-core/websites`: terp.network,
permissionless.money, DAO DAO UI (pnpm workspace: apps and packages are
members), Terp docs, Permissionless docs, Steward and WD colours (no
dependencies). `TERP_WEBSITES_DIR` overrides the websites directory. Left out:
cosmes (a library, not a front end), terp.network-stack-visuals (a copy of
terp.network's manifest) and pir-docs (another project). A root without a
lockfile is read from `package.json` alone (versions stay as ranges, source
`unlocked`).

## Output

`{ generated_at, generator, edge_format, roots[], counts{packages,edges,shared}, packages[], edges[] }`.
A package is `{ n, v, s: workspace|registry|unlocked, r: { rootId: depth }, x?: { rootId: 1 } }`
(`x` marks dev-only), keyed by name@version (workspace members are per root).
An edge is `[from, to, rootBitmask, devOnly]`. A root is
`{ id, label, bit, url, lockfile, commit, members[], counts }`. The output
carries no filesystem paths.

The Terp Network site consumes it as `public/graphs/js-deps.json` for
`public/graphs/js-deps.html`; its `scripts/gen-js-deps.mjs` calls this script
with that output path.
