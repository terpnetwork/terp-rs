#!/usr/bin/env python3
"""Build the symbient wiki timeline data (websites/svg-collections/data/symbient-days.json).

  site_data.py --cache psychedex-cache-v2.json [--records DIR ...] [--queue submissions/] --start 2026-10-16 --out symbient-days.json

Days with a finalized day-record.json are "recorded" (their record is authoritative). Other cache days are
"planned": picks + practice from the cache, pairwise interactions among the day's own picks only (memory not
applied yet). No doses anywhere; severity, standing and source travel with every interaction row.
"""
import argparse, datetime, json, os, sys
sys.path.insert(0, os.path.dirname(__file__))
import daily

DISCLAIMER = ("Fictional generative art. Not dosing, safety or medical advice. Interaction labels are AI-assembled data from "
              "psychedex.org shown with their review standing; 'unknown' means unclassified and 'unrated' means documented "
              "without a grade - neither is ever safe.")

def sub_view(s): return {k: s.get(k) for k in ("slug", "name", "url", "classes", "acute_toxicity", "effects", "page_updated")}

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cache", required=True); ap.add_argument("--records", nargs="*", default=[])
    ap.add_argument("--queue"); ap.add_argument("--start", required=True); ap.add_argument("--out", required=True)
    a = ap.parse_args()
    c = json.load(open(a.cache))
    recs = {}
    for d in a.records:
        for root, _, files in os.walk(d):
            if "day-record.json" in files:
                r = json.load(open(os.path.join(root, "day-record.json"))); recs[r["day"]] = r
    subs = []
    if a.queue and os.path.isdir(a.queue):
        for f in sorted(os.listdir(a.queue)):
            if f.endswith(".json"):
                s = json.load(open(os.path.join(a.queue, f)))
                if s.get("status") == "approved" and not daily.prefilter(s["text"]):
                    subs.append({k: s.get(k) for k in ("id", "target_day", "text", "paid_at", "status")})
    start = datetime.date.fromisoformat(a.start)
    days = []
    for d in c["days"]:
        date = (start + datetime.timedelta(days=d["day_index"])).isoformat()
        picks = [daily.from_cache(c, s) for s in d["picks"]]
        practice = c.get("practices", {}).get(d.get("practice"))
        if date in recs:
            r = recs[date]
            row = {"status": "recorded", "record_sha256": r.get("record_sha256"), "picks": [sub_view(p) for p in r["picks"]],
                   "practice": r.get("practice"), "interactions": r["interactions"], "practice_interactions": r.get("practice_interactions", []),
                   "rich_cids": [f.get("rich_cid") for f in r.get("frames", [])], "flicker": r.get("flicker"), "schema": r.get("schema")}
        else:
            inters = [daily.interaction(x, y) for i, x in enumerate(picks) for y in picks[i + 1:]]
            row = {"status": "planned", "picks": [sub_view(p) for p in picks],
                   "practice": {k: practice[k] for k in ("slug", "name", "category", "url", "effects")} if practice else None,
                   "interactions": inters, "practice_interactions": daily.practice_interactions(practice, picks) if practice else []}
        row.update({"day_index": d["day_index"], "date": date, "submissions": [s for s in subs if s["target_day"] == date]})
        days.append(row)
    out = {"schema": "symbient-site-days/v1", "cache_sha256": c["cache_sha256"], "filters": c.get("filters"),
           "disclaimer": DISCLAIMER, "days": days}
    json.dump(out, open(a.out, "w"), indent=1, ensure_ascii=False)
    print(len(days), "days;", sum(1 for x in days if x["status"] == "recorded"), "recorded ->", a.out)

if __name__ == "__main__":
    main()
