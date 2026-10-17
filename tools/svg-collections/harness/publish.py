#!/usr/bin/env python3
"""Plan (and, only with explicit flags, perform) a publish to the Terp MinIO+IPFS bucket. DRY-RUN BY DEFAULT.

Target found on groot2 (read-only recon, no credentials read):
  - S3 API (MinIO, path-style): https://s3.terp.network  (alias host minio.terp.network) -> 127.0.0.1:9000 on groot2
  - Auto-pinned buckets (webhook-collector default AUTOPIN_BUCKETS): snapshots, static, upgrades, releases
  - Pinning: every ObjectCreated in those buckets is added to kubo with cid-version=1, raw-leaves=true, pin=true;
    the CID is recorded in a SQLite pin registry and merged into <bucket>/metadata.json (keyed by object BASENAME).
  - Public IPFS gateway: https://ipfs.terp.network/ipfs/<cid>   (nginx -> kubo gateway 127.0.0.1:8081)
  - terp.network itself serves s3://static/terp.network/dist/... ; this tool uses a separate prefix.

  publish.py --site ../../../websites/svg-collections --cas /path/to/cas [--bucket static] [--prefix svg-collections]
  publish.py ... --execute --mc-alias <alias> --i-have-approval     (performs `mc cp`; credentials stay inside mc)

The CID printed for files <= 256 KiB is exact (single raw block, same as the autopin settings). Larger files are
chunked by kubo; their CID is only known after pinning (read it from metadata.json).
"""
import argparse, base64, hashlib, json, os, subprocess, sys

BLOCK = 262144
def cid_raw(data: bytes):
    if len(data) > BLOCK: return None
    return "b" + base64.b32encode(bytes([0x01, 0x55, 0x12, 0x20]) + hashlib.sha256(data).digest()).decode().lower().rstrip("=")

SKIP_DIRS = {"dev", "node_modules", ".git"}
SKIP_FILES = {"local.json"}            # local test config never ships

def plan(a):
    items = []
    if a.site:
        for root, dirs, files in os.walk(a.site):
            dirs[:] = sorted(d for d in dirs if d not in SKIP_DIRS)
            for f in sorted(files):
                if f in SKIP_FILES or ".bak-" in f or f.startswith("."): continue
                p = os.path.join(root, f); rel = os.path.relpath(p, a.site).replace(os.sep, "/")
                items.append((p, f"{a.prefix}/site/{rel}"))
    if a.cas:
        for f in sorted(os.listdir(a.cas)):
            p = os.path.join(a.cas, f)
            if os.path.isfile(p) and f.startswith("baf"): items.append((p, f"{a.prefix}/cas/{f}"))
    out, names = [], {}
    for p, key in items:
        b = open(p, "rb").read(); cid = cid_raw(b)
        if key.split("/")[-1].startswith("baf") and cid and cid != key.split("/")[-1]:
            raise SystemExit(f"CAS file name does not match its content CID: {p}")
        base = key.rsplit("/", 1)[-1]; names.setdefault(base, []).append(key)
        out.append({"local": p, "bytes": len(b), "sha256": hashlib.sha256(b).hexdigest(), "bucket": a.bucket, "key": key,
                    "s3_url": f"{a.endpoint.rstrip('/')}/{a.bucket}/{key}", "cid_v1_raw": cid,
                    "ipfs_url": f"{a.gateway.rstrip('/')}/{cid}" if cid else "(chunked: CID assigned by kubo at pin time)"})
    collisions = {k: v for k, v in names.items() if len(v) > 1}
    return out, collisions

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--site"); ap.add_argument("--cas")
    ap.add_argument("--bucket", default="static"); ap.add_argument("--prefix", default="svg-collections")
    ap.add_argument("--endpoint", default="https://s3.terp.network"); ap.add_argument("--gateway", default="https://ipfs.terp.network/ipfs")
    ap.add_argument("--json", help="write the plan here")
    ap.add_argument("--execute", action="store_true"); ap.add_argument("--mc-alias"); ap.add_argument("--i-have-approval", action="store_true")
    a = ap.parse_args()
    if a.bucket not in {"snapshots", "static", "upgrades", "releases"}:
        print(f"warning: bucket {a.bucket!r} is not in the default AUTOPIN_BUCKETS; objects would not be pinned", file=sys.stderr)
    items, collisions = plan(a)
    total = sum(i["bytes"] for i in items)
    print(f"{'DRY RUN' if not a.execute else 'EXECUTE'}: {len(items)} objects, {total:,} bytes -> s3://{a.bucket}/{a.prefix}/ via {a.endpoint}")
    for i in items:
        print(f"  PUT {i['key']}  ({i['bytes']:,} B)  ipfs: {i['cid_v1_raw'] or 'chunked'}")
    if collisions:
        print(f"note: {len(collisions)} basenames repeat (e.g. {next(iter(collisions))}); the autopin metadata.json index is keyed "
              "by basename, so only the last one keeps an index entry. Pins themselves are unaffected.")
    if a.json: json.dump({"items": items, "basename_collisions": collisions}, open(a.json, "w"), indent=1)
    if not a.execute:
        print("nothing uploaded (dry run). To publish: --execute --mc-alias <alias> --i-have-approval"); return
    if not (a.mc_alias and a.i_have_approval):
        raise SystemExit("refusing: --execute needs --mc-alias and --i-have-approval")
    for i in items:   # credentials stay inside the operator's mc alias; this script never reads them
        subprocess.run(["mc", "cp", "--quiet", i["local"], f"{a.mc_alias}/{a.bucket}/{i['key']}"], check=True)
    print("uploaded; verify pins via", f"{a.endpoint}/{a.bucket}/metadata.json", "and", a.gateway)

if __name__ == "__main__":
    main()
