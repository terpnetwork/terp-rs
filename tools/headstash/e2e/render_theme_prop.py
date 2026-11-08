#!/usr/bin/env python3
"""R12: render a DAO proposal that calls the owner-only cw-headstash set_theme on each target (no tx is sent).

  python3 tools/headstash/e2e/render_theme_prop.py --theme tools/headstash/frontend/themes/azure-calcium.json \
    --target terpy=terp16ww6... --target thioly=terp1yc4q... \
    --out tools/headstash/deploy/rendered-mainnet/dao-flow/52-dao-prop-headstash-theme.propose.json
"""
import argparse, base64, hashlib, json, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import render_frontend_prop as rf

def theme_sha(t): return hashlib.sha256(json.dumps(t, separators=(",", ":")).encode()).hexdigest()

def body(theme_file, targets):
    j = json.load(open(theme_file)); t = rf.canon_theme(j.get("theme", j)); name = j.get("name", os.path.basename(theme_file)); src = j.get("source", "")
    names = " and ".join("%s %s" % (n, a) for n, a in targets)
    desc = ("Sets the Headstash claim page theme (R12) on the DAO-owned cw-headstash instances %s via the owner-only set_theme. "
            "Theme: %s%s, light and dark token sets (%s), canonical JSON sha256 %s. The contract validates every token (#RRGGBB, no unknown fields); "
            "the page validates again and falls back to the frontend manifest default, then the built-in theme. No funds move. Reverting is set_theme {theme: null}."
            % (names, name, " (%s)" % src if src else "", ", ".join(rf.THEME_TOKENS), theme_sha(t)))
    msg = base64.b64encode(json.dumps({"set_theme": {"theme": t}}, separators=(",", ":")).encode()).decode()
    msgs = [{"wasm": {"execute": {"contract_addr": a, "msg": msg, "funds": []}}} for _, a in targets]
    title = "Headstash claim page: set %s theme on %s" % (name, " + ".join(n for n, _ in targets))
    return {"propose": {"msg": {"propose": {"title": title, "description": desc, "msgs": msgs}}}}, t

def main():
    ap = argparse.ArgumentParser(); ap.add_argument("--theme", required=True); ap.add_argument("--target", action="append", required=True); ap.add_argument("--out", required=True)
    a = ap.parse_args(); targets = [tuple(x.split("=", 1)) for x in a.target]
    b, t = body(a.theme, targets)
    with open(a.out, "w") as f: json.dump(b, f, indent=1); f.write("\n")
    print(json.dumps({"out": a.out, "sha256": hashlib.sha256(open(a.out, "rb").read()).hexdigest(), "theme_sha256": theme_sha(t)}, indent=1))

if __name__ == "__main__":
    main()
