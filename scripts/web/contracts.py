#!/usr/bin/env python3
"""Serve the contract page from the terp-rs root, and check the ESM bundles.

The page imports /api/ts/<id>/esbundles/*.mjs. Those files sit next to this
workspace, not inside web/. A server whose root is web/ returns 404 for every
bundle, and the browser reports that as a failed dynamic import.

State files are read from a fixed list. The default is
~/.cw-orchestrator/state.json. The KMS deploy scripts write
~/.cw-orch/state.json. The index at /states names each file and the chains
that have code ids or addresses, so a file can be told apart before it is
loaded. Backup copies and Cargo incremental state.json files are not on the
list. GET /ibc returns the predetermined token routes and the curated
counterparty channels.
"""

import argparse
import json
import mimetypes
import re
import subprocess
import sys
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import unquote, urlparse

ROOT = Path(__file__).resolve().parents[2]
CORE = ROOT.parents[1]
HOME = Path.home() / ".cw-orchestrator"
KMS = Path.home() / ".cw-orch"
ARTIFACTS = CORE / "artifacts" / "community-core-local"

# id, path, label shown in the page.
SOURCES = (
    ("cw-orch", HOME / "state.json", "cw-orch ~/.cw-orchestrator/state.json"),
    ("cw-orch-kms", KMS / "state.json", "kms ~/.cw-orch/state.json"),
    ("cw-orch-local", HOME / "state_local.json", "local ~/.cw-orchestrator/state_local.json"),
    ("cw-orch-120u", HOME / "state-120u-1-v6.json", "120u ~/.cw-orchestrator/state-120u-1-v6.json"),
    ("repo-public", ROOT / "public" / "state.json", "repo public/state.json"),
    ("artifact-120u", ARTIFACTS / "state.120u-1-v6.json", "artifacts state.120u-1-v6.json"),
    (
        "artifact-morocco",
        ARTIFACTS / "state.morocco-1-ibc-poly.json",
        "artifacts state.morocco-1-ibc-poly.json",
    ),
    (
        "vote11-stray",
        ROOT / "out" / "mainnet-deploy" / "state.vote11-stray.json",
        "out/mainnet-deploy/state.vote11-stray.json",
    ),
)
BY_ID = {item[0]: item for item in SOURCES}

# Catalog id to the contracts/ directory name when they differ.
SCHEMA_DIRS = {
    "cw-headstash-manifold": "manifold",
    "cw-oracle-aggregator": "oracle-aggregator",
    "cw-oracle-registry": "oracle-registry",
    "cw-oracle-twap-probe": "oracle-twap-probe",
    "cw-private-dex": "private-dex",
    "cw-svg-minter": "cw721-svg-minter",
    "nftfi-loan": "nft-loan",
    "nftfi-raffle": "raffle",
    "revenue-marketplace": "marketplace",
    "revenue-reserve-auction": "reserve-auction",
    "whitelist-mtree": "whitelist-merkletree",
}

mimetypes.add_type("text/javascript", ".mjs")


def chain_rows(doc):
    if not isinstance(doc, dict):
        return []
    rows = []
    for key, value in doc.items():
        if not isinstance(value, dict):
            continue
        if "code_ids" not in value and "default" not in value:
            continue
        codes = value.get("code_ids") or {}
        addrs = value.get("default") or {}
        rows.append(
            {
                "id": key,
                "codes": len(codes) if isinstance(codes, dict) else 0,
                "addresses": len(addrs) if isinstance(addrs, dict) else 0,
            }
        )
    return rows


def index():
    out = []
    for sid, path, label in SOURCES:
        present = path.is_file()
        chains = []
        if present:
            try:
                chains = chain_rows(json.loads(path.read_text()))
            except json.JSONDecodeError:
                present = False
        out.append(
            {
                "id": sid,
                "label": label,
                "present": present,
                "chains": chains,
            }
        )
    return out


def contract_schema_dirs():
    found = {}
    for schema in (ROOT / "contracts").rglob("schema"):
        if not schema.is_dir():
            continue
        found.setdefault(schema.parent.name, schema.parent)
    return found


SCHEMA_ROOTS = contract_schema_dirs()


def schema_pair(contract_id):
    directory = SCHEMA_ROOTS.get(SCHEMA_DIRS.get(contract_id, contract_id))
    if directory is None:
        return None
    raw_query = directory / "schema" / "raw" / "query.json"
    raw_execute = directory / "schema" / "raw" / "execute.json"
    if raw_query.is_file():
        query = json.loads(raw_query.read_text())
        execute = json.loads(raw_execute.read_text()) if raw_execute.is_file() else None
        return {"query": query, "execute": execute}
    for path in sorted((directory / "schema").glob("*.json")):
        doc = json.loads(path.read_text())
        if isinstance(doc, dict) and isinstance(doc.get("query"), dict):
            return {"query": doc.get("query"), "execute": doc.get("execute")}
    return None


def ibc_view():
    public = ROOT / "public"
    denoms_path = public / "ibc-accepted" / "accepted-denoms.json"
    tokens = json.loads(denoms_path.read_text()) if denoms_path.is_file() else {}
    paths = []
    data = public / "ibc-data"
    if data.is_dir():
        for path in sorted(data.glob("*.json")):
            doc = json.loads(path.read_text())
            paths.append(
                {
                    "id": path.stem,
                    "client_status": doc.get("client_status"),
                    "chain_1": doc.get("chain_1"),
                    "chain_2": doc.get("chain_2"),
                    "channels": doc.get("channels") or [],
                }
            )
    return {"tokens": tokens, "paths": paths}


def catalog_bundles():
    text = (ROOT / "web" / "catalog.js").read_text()
    return re.findall(r'bundle:\s*"([^"]+)"', text)


def check():
    paths = catalog_bundles()
    if len(paths) < 1:
        print("catalog.js has no bundle paths", file=sys.stderr)
        return 1
    bad = []
    files = []
    for rel in paths:
        if not rel.startswith("/api/ts/") or ".." in rel:
            bad.append(f"path is not under /api/ts: {rel}")
            continue
        file = ROOT / rel.lstrip("/")
        if not file.is_file():
            bad.append(f"missing {rel}")
            continue
        files.append(file.resolve().as_uri())
    ids = re.findall(r'id:\s*"([^"]+)"', (ROOT / "web" / "catalog.js").read_text())
    missing_schema = [cid for cid in ids if schema_pair(cid) is None]
    if missing_schema:
        bad.extend(f"no schema for {cid}" for cid in missing_schema)
    if bad:
        print("\n".join(bad), file=sys.stderr)
        return 1
    script = r"""
const paths = process.argv.slice(1);
let bad = 0;
for (const href of paths) {
  const mod = await import(href);
  if (typeof mod.createApi !== "function" || !mod.Query || typeof mod.Query !== "object") {
    console.error("bundle missing createApi or Query: " + href);
    bad += 1;
  }
}
if (bad) process.exit(1);
console.log(paths.length + " bundles export createApi and Query");
"""
    result = subprocess.run(
        ["node", "--input-type=module", "-e", script, *files],
        check=False,
    )
    if result.returncode == 0:
        print(f"{len(ids)} schemas")
    return result.returncode


class Handler(SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(ROOT), **kwargs)

    def do_GET(self):
        path = unquote(urlparse(self.path).path)
        if path == "/":
            self.send_response(302)
            self.send_header("Location", "/web/")
            self.end_headers()
            return
        if path == "/states":
            self._json(index())
            return
        if path == "/ibc":
            self._json(ibc_view())
            return
        if path.startswith("/schemas/"):
            sid = path[len("/schemas/") :]
            pair = schema_pair(sid)
            if pair is None:
                self.send_error(404, "no schema for this contract")
                return
            self._json(pair)
            return
        if path.startswith("/states/"):
            sid = path[len("/states/") :]
            item = BY_ID.get(sid)
            if item is None or not item[1].is_file():
                self.send_error(404, "unknown state file")
                return
            try:
                body = json.loads(item[1].read_text())
            except json.JSONDecodeError:
                self.send_error(404, "state file is not json")
                return
            self._json(body)
            return
        super().do_GET()

    def _json(self, body):
        raw = json.dumps(body).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)


def serve(port):
    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    print(f"terp-rs contract page http://127.0.0.1:{port}/web/", flush=True)
    print("state index http://127.0.0.1:%s/states" % port, flush=True)
    print("ibc routes http://127.0.0.1:%s/ibc" % port, flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print()
    finally:
        server.server_close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--serve", action="store_true")
    parser.add_argument("--port", type=int, default=4173)
    args = parser.parse_args()
    if args.check == args.serve:
        parser.error("pass one of --check or --serve")
    if args.check:
        sys.exit(check())
    serve(args.port)


if __name__ == "__main__":
    main()
