#!/usr/bin/env python3
"""Rebuild the Latent Benders study files from finished day records and upload them.

The page reads data/latent-benders.json. A new on-chain day does not show its
Kara and Gippy session until that file, config/mainnet.json, and the frame
HTML are on the host that serves the site. This is the only writer.
"""
import json, os, shutil, subprocess, sys, time
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
    import hashlib
    import urllib.request
    ok = True
    pending = []
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
        pending.append((local, key))
    if not pending:
        return ok
    if not upload_on_groot2(pending):
        return False
    for _, key in pending:
        print(f"uploaded {key}")
    return ok


def upload_on_groot2(pending):
    """Put objects with the silo root on groot2. The SVG mc aliases are not valid there."""
    stage = Path("/tmp/svgc-publish-stage")
    if stage.exists():
        shutil.rmtree(stage)
    lines = []
    for local, key in pending:
        dest = stage / key
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(local, dest)
        if key.endswith(".html"):
            attr = "Content-Type=text/html; charset=utf-8"
        elif key.endswith(".json"):
            attr = "Content-Type=application/json"
        elif key.endswith(".js"):
            attr = "Content-Type=text/javascript; charset=utf-8"
        else:
            attr = "Content-Type=application/octet-stream"
        lines.append(f"{key}\t{attr}")
    (stage / "MANIFEST").write_text("\n".join(lines) + "\n")
    scp = None
    for attempt in range(1, 4):
        scp = subprocess.run(
            ["scp", "-o", "BatchMode=yes", "-o", "ConnectTimeout=20", "-r", str(stage), "groot2:/tmp/svgc-publish-stage"],
            capture_output=True, text=True)
        if scp.returncode == 0:
            break
        err = (scp.stderr or scp.stdout).strip().splitlines()
        print(f"upload attempt {attempt} failed: {err[-1] if err else scp.returncode}", file=sys.stderr)
        time.sleep(5)
    if scp is None or scp.returncode != 0:
        return False
    remote = r"""
set -u
export PATH="$PATH:/usr/local/bin:$HOME/bin:/usr/bin"
read_cred() { if [ -r "$1" ]; then cat "$1"; elif sudo -n test -r "$1" 2>/dev/null; then sudo -n cat "$1"; else return 1; fi; }
user=$(read_cred /etc/silo/root-user) || { echo "root-user unreadable" >&2; exit 1; }
pass=$(read_cred /etc/silo/root-password) || { echo "root-password unreadable" >&2; exit 1; }
user=${user%$'\n'}; pass=${pass%$'\n'}
mc alias set silo-root http://127.0.0.1:9000 "$user" "$pass" >/dev/null
unset user pass
fail=0
while IFS=$(printf '\t') read -r key attr; do
  [ -n "$key" ] || continue
  mc cp --attr "$attr" "/tmp/svgc-publish-stage/$key" "silo-root/$key" >/dev/null || fail=1
done < /tmp/svgc-publish-stage/MANIFEST
mc alias rm silo-root >/dev/null 2>&1 || true
rm -rf /tmp/svgc-publish-stage
exit $fail
"""
    ssh = subprocess.run(
        ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=40", "groot2", "bash", "-s"],
        input=remote, capture_output=True, text=True)
    shutil.rmtree(stage, ignore_errors=True)
    if ssh.returncode != 0:
        err = (ssh.stderr or ssh.stdout).strip().splitlines()
        print(f"upload failed: {err[-1] if err else ssh.returncode}", file=sys.stderr)
        return False
    return True


def main():
    written = rebuild()
    for day, addr in written:
        print(f"study {day} {addr}")
    site = SITE
    # Only the study index, the config map, and frames named by those days.
    # Shipping every historical cas file made scp drop the connection, so the
    # new day's study never followed the collection.
    paths = []
    for prefix in (LIVE, "static/svg-collections"):
        paths.append((site / "data" / "latent-benders.json", f"{prefix}/data/latent-benders.json"))
        paths.append((site / "config" / "mainnet.json", f"{prefix}/config/mainnet.json"))
        paths.append((site / "studies" / "latent-benders.json", f"{prefix}/studies/latent-benders.json"))
    data = json.loads((site / "data" / "latent-benders.json").read_text())
    wanted = {day for day, _ in written}
    for row in data.get("days") or []:
        if row.get("day") not in wanted:
            continue
        for cid in row.get("frames") or []:
            html = site / "cas" / f"{cid}.html"
            if not html.exists():
                print(f"missing frame {cid}", file=sys.stderr)
                continue
            paths.append((html, f"{CAS}/{cid}.html"))
            paths.append((html, f"{LIVE}/cas/{cid}.html"))
    # A zero exit with the study still local is what left Day 7 off the site.
    if os.environ.get("SVGC_PUBLISH") == "1" and not upload(paths):
        sys.exit(1)


if __name__ == "__main__":
    main()
