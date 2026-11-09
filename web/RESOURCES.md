# Later: contracts on the resources page

Do not move this page yet. When it lands, it lands inside the existing App state section of the terp.network resources page. It does not become a second site.

## Where it goes

- Page: `websites/terp.network/pages/snapshots.html`, hash `#app-state`.
- Host: `#app-state-host`, mounted by `lib/app-state.js`.
- The section already reads the published file `https://s3.terp.network/snapshots/mainnet/morocco-1/state.json` and groups rows by category. A contract query opens from one of those rows. It does not replace the category picker.
- The fractal nav already owns the section (`sec:app-state`). On a panel page `#res-subtabs` is hidden. Do not add another left rail.

## What to keep from resources

`public/world.css` is the design. On `html[data-tn-world="panel"]` cards, notes, and code blocks have no fill. Text uses `--tnw-halo`. Inputs are transparent. Actions use `.snap-btn`. Selection is a wash and a ring in the section colour, not a side bar and not a filled card. The local page’s paper background and white cards stay on this tool. They do not ship.

## What this tool adds

- The list is the code ids in the open state file. Names with no code id are not rows.
- A query or execute body is the required fields from that contract’s schema (`schema/raw/query.json` and `execute.json`, or the `query` / `execute` objects in `schema/<name>.json`). The browser wraps that object as `{ "<variant>": <body> }`. An empty `{}` is what produced `500 kind: Serialization`.
- `just web serve` and `~/.cw-orchestrator/state.json` stay local. The site uses the published state file only.

## What has to be published first

`api/` is gitignored, and the schemas live under `contracts/`. The static site cannot import either until a build copies, per catalog id:

- `api/ts/<id>/esbundles/*.bundle.mjs`
- the matching `query` and `execute` schema

Same-origin paths, the way `/lib/app-state.js` is same-origin today. A server rooted in a page directory 404s those imports.

Keplr stays off until someone connects. The first action on the public row is a query.

The Paths tab stays with this tool until that move. It reads `GET /ibc`, which is `public/ibc-accepted/accepted-denoms.json` plus `public/ibc-data/*.json`. A contract row asks the REST node for `GET /cosmwasm/wasm/v1/code/{code_id}/contracts` and pages with `pagination.limit` and `pagination.key`. Do not send `pagination.count_total`; this node rejects it. The published resources page would use the same calls against `https://api.terp.network`, inside `#app-state-host`, without a second rail.
