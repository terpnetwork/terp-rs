#!/usr/bin/env python3
"""Rebuild the Latent Benders study files from finished day records and upload them.

The page reads data/latent-benders.json. A new on-chain day does not show its
Kara and Gippy session until that file, config/mainnet.json, and the frame
HTML are on the host that serves the site. This is the only writer.
"""
import json, os, shutil, subprocess, sys
from pathlib import Path

STATE = Path(os.environ.get("SVGC_STATE", Path.home() / "Library/Application Support/terp-svg-collections"))
SITE = Path(os.environ.get("SVGC_SITE", Path(__file__).resolve().parents[4] / "websites" / "svg-collections"))
# parents: harness, svg-collections, tools, terp-rs. Website is terp-core/websites.
if not (SITE / "data" / "latent-benders.json").exists():
    SITE = Path(__file__).resolve().parents[5] / "websites" / "svg-collections"
# permissionless.money/svg is served from this prefix. The old
# static/permissionless.money/dist/svg/ copy is not the live site.
LIVE = os.environ.get("SVGC_LIVE_PREFIX", "static/svg-collections/pm")
# Frame links use this gateway, including the pm pages.
CAS = os.environ.get("SVGC_CAS_PREFIX", "static/svg-collections/cas")
ALIAS = os.environ.get("SVGC_MC_ALIAS", "")


def records():
    root = STATE / "days"
    if not root.exists():
        return
    for day_dir in sorted(p for p in root.iterdir() if p.is_dir()):
        rec_path, chain_path = day_dir / "day-record.json", day_dir / "onchain.json"
        if rec_path.exists() and chain_path.exists():
            rec = json.loads(rec_path.read_text())
            addr = chain_path.read_text().strip().split()[-1]
            if addr.startswith("terp1"):
                yield day_dir, rec, addr


def row_for(rec, addr):
    sess = rec["session"]
    cids = [f["rich_cid"] for f in rec["frames"]]
    return {
        "day": rec["day"], "day_index": rec["day_index"], "contract": addr,
        "title": f"Latent Benders: A Study - {rec['day']}",
        "record_sha256": rec["record_sha256"], "picks": rec["picks"],
        "practice": rec["practice"], "interactions": rec["interactions"],
        "practice_interactions": rec.get("practice_interactions") or [],
        "draw": rec["draw"], "transcript": sess["transcript"], "painting": sess["painting"],
        "palette": sess["palette"], "frames": cids, "onchain_frame_cids": cids,
        "model": rec["model"],
    }


def rebuild():
    data_path = SITE / "data" / "latent-benders.json"
    data = json.loads(data_path.read_text())
    cfg_path = SITE / "config" / "mainnet.json"
    cfg = json.loads(cfg_path.read_text())
    by_day = {d["day"]: d for d in data["days"]}
    used = set(data.get("used_substances") or [])
    written = []
    for day_dir, rec, addr in records():
        row = row_for(rec, addr)
        by_day[row["day"]] = row
        used.update(p["slug"] for p in rec["picks"])
        cfg["collections"]["latentBenders"][row["day"]] = addr
        for cid in row["frames"]:
            src = STATE / "cas" / cid
            if src.exists():
                shutil.copyfile(src, SITE / "cas" / f"{cid}.html")
        written.append((row["day"], addr))
    data["days"] = [by_day[k] for k in sorted(by_day)]
    data["used_substances"] = sorted(used)
    data_path.write_text(json.dumps(data, indent=1, ensure_ascii=False) + "\n")
    cfg_path.write_text(json.dumps(cfg, indent=1, ensure_ascii=False) + "\n")
    overlay = {"days": [by_day[k] for k in sorted(by_day) if k >= "2026-10-28"]}
    studies = SITE / "studies"
    studies.mkdir(exist_ok=True)
    (studies / "latent-benders.json").write_text(json.dumps(overlay, indent=1, ensure_ascii=False) + "\n")
    return written


def upload(paths):
    if not ALIAS:
        print("study files updated locally; SVGC_MC_ALIAS is unset so the live bucket was not changed")
        return False
    import hashlib
    import urllib.request
    ok = True
    for local, key in paths:
        body = local.read_bytes()
        digest = hashlib.sha256(body).hexdigest()
        url = f"https://s3.terp.network/{key}"
        try:
            with urllib.request.urlopen(url, timeout=30) as resp:
                remote_body = resp.read()
            # The public edge appends a Cloudflare beacon to HTML responses.
            marker = b'<script type="module" src="https://static.cloudflareinsights.com/'
            cut = remote_body.find(marker)
            if cut != -1:
                remote_body = remote_body[:cut] + b"</body></html>"
            remote = hashlib.sha256(remote_body).hexdigest()
        except Exception:
            remote = None
        if remote == digest:
            print(f"unchanged {key}")
            continue
        # CAS objects are content-addressed. A different body at the same key is a mistake.
        if remote is not None and "/cas/" in key:
            print(f"refusing to replace {key}", file=sys.stderr)
            ok = False
            continue
        dest = f"{ALIAS}/{key}"
        proc = subprocess.run(["mc", "cp", str(local), dest], capture_output=True, text=True)
        if proc.returncode != 0:
            ok = False
            err = (proc.stderr or proc.stdout).strip().splitlines()
            print(f"upload failed {key}: {err[-1] if err else proc.returncode}", file=sys.stderr)
        else:
            print(f"uploaded {key}")
    return ok


def main():
    written = rebuild()
    for day, addr in written:
        print(f"study {day} {addr}")
    site = SITE
    paths = [
        (site / "data" / "latent-benders.json", f"{LIVE}/data/latent-benders.json"),
        (site / "config" / "mainnet.json", f"{LIVE}/config/mainnet.json"),
        (site / "studies" / "latent-benders.json", f"{LIVE}/studies/latent-benders.json"),
    ]
    # The pm page build is not this repo file. Do not overwrite it from here.
    for html in (site / "cas").glob("baf*.html"):
        paths.append((html, f"{CAS}/{html.name}"))
        paths.append((html, f"{LIVE}/cas/{html.name}"))
    # The static bucket key that permissionless.money/svg/ actually serves.
    if os.environ.get("SVGC_PUBLISH") == "1":
        upload(paths)


if __name__ == "__main__":
    main()
