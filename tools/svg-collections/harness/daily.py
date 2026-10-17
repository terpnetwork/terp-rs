#!/usr/bin/env python3
"""Daily curation harness (stdlib only). Talks to a LOCAL open model (ollama) on 127.0.0.1.

  daily.py marain-draft  --out poems-draft.json [--n 24]
  daily.py build-cache   --out psychedex-cache-v1.json [--days 30]     (one-off, polite crawl)
  daily.py commit       --day YYYY-MM-DD --state state.json          (BEFORE the day: commit sha256(seed))
  daily.py symbient-day  --persona personas/<id>.json --day YYYY-MM-DD --state state.json --out DIR
                         [--source cache|live] [--cache psychedex-cache-v1.json] [--queue submissions/] [--frames 3]

Filters (iteration 3, user decision): the substance blocklist and the acute-toxicity filter are OFF by
default - the symbient is an uncensored documentation of effects. `--filters` re-enables both (opt-in).
Doses are never parsed, stored or shown; severity, review standing and source are always shown;
'unknown' / 'unrated' is never presented as safe.

Iteration 4 ("Latent Benders: A Study"): every day is drawn FRESH. There is no memory and no carry-over
in selection or art. The draw uses a commit-reveal seed: `commit` writes a random 32-byte seed to
seeds/<day>.seed and records sha256(seed) in state.json before the day; `symbient-day` reveals it, checks
the hash, and records both in the day record. A substance that appeared on an earlier day is never drawn
again (state.used_substances); the run stops with an exhaustion report when the pool runs out.
The fictional/safety disclaimer is NOT part of prompts, art, narration or records; it lives on the website.

Each day pairs the substance pick(s) with one psychedex *practice* (meditation, breathwork, movement...).
Practice<->substance interactions are recorded only where the practice page's "Substance Interactions"
or "Contraindications" prose names the substance or one of its classes. Practice pages carry no graded
severity, so those rows are `severity: "unrated"` with the sentence they come from.

Until psychedex licensing is settled the default --source is `cache`: no network, picks come
from the pre-built cache (fields the harness needs + source URL only, never doses).

No scheduling, no chain txs: the harness writes files. Committing the record hash on chain
(as the collection `seed`) is a separate, human-approved step.
"""
import argparse, hashlib, html, json, os, re, shutil, struct, subprocess, tempfile, time, urllib.request

OLLAMA = os.environ.get("OLLAMA_URL", "http://127.0.0.1:11535")
MODEL = os.environ.get("SVGC_MODEL", "qwen3.5:0.8b")
SVGC = os.environ.get("SVGC_BIN", "svgc")
MAX_ATTEMPTS = 4
PDX = "https://psychedex.org"
UA = "terp-svg-collections/0.1 (+art project; public pages only)"
SEV = ["lethal", "dangerous", "caution", "low risk", "low_risk", "no known interaction"]
SEV_RANK = {"lethal": 0, "dangerous": 1, "caution": 2, "low risk": 3, "low_risk": 3}
PERSONA_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "personas")
KARA_PALETTE = ["#5f6b66", "#8a9a8c", "#d8d2c4"]   # steady, never changes
CREDIT = "Kara &amp; Gippy are original characters of this project, inspired by wibandwob's symbient approach."

def primer(name):
    """Stance + voice sections of a persona primer (the visual section is for the art, not the prompt)."""
    t = open(os.path.join(PERSONA_DIR, name)).read()
    keep, on = [], False
    for l in t.splitlines():
        if l.startswith("**Role.**") or l.startswith("**Stance.**") or l.startswith("**Voice rules.**"): on = True
        elif l.startswith("**Visual language.**") or l.startswith("# "): on = False
        if on and l.strip(): keep.append(re.sub(r"\s*\(\"[^)]*\)", "", l.replace("**", "")))  # drop quoted examples (the small model parrots them)
    return "\n".join(keep)

def gippy_hue(seed_hex): return int(seed_hex[:4], 16) % 360

DOSE_RE = re.compile(r"\b\d+(\.\d+)?\s?(mg|mcg|ug|µg|g|ml|tabs?|hits?|drops?|grams?|milligrams?|micrograms?)\b", re.I)

def sha(b): return hashlib.sha256(b if isinstance(b, bytes) else b.encode()).hexdigest()
def canon(o): return json.dumps(o, sort_keys=True, separators=(",", ":"), ensure_ascii=False)

# ---------------- ollama ----------------
def ollama(prompt, seed, temperature=0.8, num_predict=1400):
    body = {"model": MODEL, "prompt": prompt, "stream": False, "think": False,
            "options": {"seed": seed, "temperature": temperature, "num_predict": num_predict}}
    req = urllib.request.Request(OLLAMA + "/api/generate", data=json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=600) as r:
        out = json.load(r)
    return out["response"], body["options"]

def model_info():
    with urllib.request.urlopen(OLLAMA + "/api/tags", timeout=30) as r:
        for m in json.load(r)["models"]:
            if m["name"] == MODEL:
                return {"runtime": "ollama", "name": MODEL, "digest": m.get("digest")}
    raise SystemExit(f"model {MODEL} not present locally")

# ---------------- psychedex (public .md pages only; never /api/) ----------------
def fetch(url, cache):
    os.makedirs(cache, exist_ok=True)
    p = os.path.join(cache, re.sub(r"[^a-zA-Z0-9.-]", "_", url.split("//", 1)[1]))
    if not os.path.exists(p):
        assert "/api/" not in url, "psychedex /api/ is disallowed by robots.txt"
        time.sleep(1.0)
        req = urllib.request.Request(url, headers={"User-Agent": UA})
        with urllib.request.urlopen(req, timeout=60) as r:
            open(p, "wb").write(r.read())
    return open(p, encoding="utf-8").read()

def substance_slugs(cache):
    txt = fetch(PDX + "/llms-full.txt", cache)
    sec = txt.split("## Substances", 1)[1].split("\n## ", 1)[0]
    return sorted(set(re.findall(r"\]\(https://psychedex\.org/substances/([a-z0-9-]+)/llms\.md\)", sec)))

def parse_substance(slug, cache):
    url = f"{PDX}/substances/{slug}.md"
    md = fetch(url, cache)
    name = md.split("\n", 1)[0].lstrip("# ").strip()
    upd = (re.search(r"^Updated: (\S+)", md, re.M) or [None, None])[1]
    cls = (re.search(r"^- Classes: (.*)$", md, re.M) or [None, ""])[1]
    classes = sorted({c.strip() for part in cls.split(";") for c in part.split(":", 1)[-1].split(",") if c.strip()})
    tox = (re.search(r"\*\*Acute toxicity\*\*: (\w+)", md) or [None, "unknown"])[1]
    if tox.lower() in ("no", "none", "unknown", "n"): tox = "unknown"     # "No data" etc. -> unknown, never safe
    inter = []
    if "## Interactions" in md:
        sec = md.split("## Interactions", 1)[1].split("\n## ", 1)[0]
        for row in re.findall(r"^\| (.+?) \| (\w[\w ]*?) \| (.+?) \| (\w+) \|$", sec, re.M):
            if row[0].startswith("Substance or class") or row[0].startswith("---"): continue
            inter.append({"label": row[0], "severity": row[1].lower(), "standing": row[2], "confidence": row[3]})
    effects = []
    if "## Subjective Effects" in md:
        sec = md.split("## Subjective Effects", 1)[1].split("\n## ", 1)[0]
        for e in re.findall(r"\[([^\]]+)\]\(https://psychedex\.org/effects/[a-z0-9-]+\)", sec):
            if e.lower() not in [x.lower() for x in effects]: effects.append(e)
    # NOTE: dose tables are deliberately never parsed or stored.
    return {"slug": slug, "name": name, "url": url, "page_updated": upd, "classes": classes,
            "acute_toxicity": tox, "effects": effects[:4], "_interactions": inter}

# ---------------- practices (public HTML pages; no .md variant exists) ----------------
DOSE_RE = re.compile(r"\b\d+(\.\d+)?\s?(mg|mcg|ug|µg|g|ml|grams?|milligrams?|micrograms?|mg/kg|µg/kg)\b", re.I)
CUE_RULES = [("risk", r"contraindicat|elevat\w* (?:the )?(?:cardiovascular )?risk|compound|unfavorabl|danger|life-threat|fatal|seizure|avoid|hazard|serotonin syndrome|hypertensi"),
             ("attenuates", r"attenuat|blunt|reduc\w* the|dampen|weaken"),
             ("adjunct", r"adjunct|integration|alongside|combined with|paired with|follow\w* (?:psilocybin|mdma|ketamine)")]

def practice_slugs(cache):
    txt = fetch(PDX + "/llms-full.txt", cache)
    sec = txt.split("## Practices", 1)[1].split("\n## ", 1)[0]
    out = []
    for name, slug, summ in re.findall(r"^- \[([^\]]+)\]\(https://psychedex\.org/practices/([a-z0-9-]+)\): (.*)$", sec, re.M):
        cat = (re.search(r"\bis an? ([\w\s,/-]+?) (?:practice|phenomenon|method)", summ) or [None, "practice"])[1]
        out.append({"slug": slug, "name": name, "category": cat.strip().lower()})
    return out

def _text(fragment):
    t = re.sub(r"<(script|style)\b.*?</\1>", " ", fragment, flags=re.S)
    t = re.sub(r"<[^>]+>", " ", t)
    t = html.unescape(t)
    t = re.sub(r"\[\d+(?:[,\u2013-]\d+)*\]", " ", t)
    return re.sub(r"\s+", " ", t).strip()

def _section(page, title):
    m = re.search(r"<h[23][^>]*>\s*" + re.escape(title) + r"\s*</h[23]>", page)
    if not m: return "", ""
    rest = page[m.end():]
    end = re.search(r"<h[23][^>]*>", rest)
    frag = rest[:end.start()] if end else rest[:20000]
    return frag, _text(frag)

def _sentences(text):
    return [x.strip() for x in re.split(r"(?<=[.;!?])\s+(?=[A-Z])", text) if x.strip()]

def parse_practice(meta, cache):
    url = f"{PDX}/practices/{meta['slug']}"
    page = fetch(url, cache)
    effects = []
    frag, _ = _section(page, "Subjective Effects")
    for e in re.findall(r'href="/effects/[a-z0-9-]+"[^>]*>(.*?)</a>', frag, flags=re.S):
        e = _text(e)
        if e and e.lower() not in [x.lower() for x in effects]: effects.append(e)
    notes, linked = [], set()
    for title in ("Substance Interactions", "Contraindications"):
        frag, text = _section(page, title)
        linked |= set(re.findall(r'href="/substances/([a-z0-9-]+)"', frag))
        for snt in _sentences(text):
            if DOSE_RE.search(snt): continue          # never keep a sentence carrying a dose
            notes.append({"section": title, "text": snt[:320]})
    # NOTE: no dose, duration or technique-protocol numbers are kept; only interaction/contraindication prose.
    return {"slug": meta["slug"], "name": meta["name"], "category": meta["category"], "url": url,
            "effects": effects[:4], "linked_substances": sorted(linked), "notes": notes}

def _cue(text):
    for name, rx in CUE_RULES:
        if re.search(rx, text, re.I): return name
    return "mention"

def practice_interactions(practice, substances):
    """Rows where the practice page's prose names the substance (or links it, or names one of its classes)."""
    rows = []
    for sub in substances:
        terms = {sub["name"].lower(), sub["slug"].replace("-", " ")}
        for c in sub["classes"]:
            c = c.lower(); terms.add(c); terms.add(c + "s")
            for k, al in ALIASES.items():
                if c in al: terms.add(k)
        terms = {t for t in terms if len(t) >= 3}
        hits = []
        for n in practice["notes"]:
            low = n["text"].lower()
            if sub["slug"] in practice["linked_substances"] and (sub["name"].lower() in low or sub["slug"] in low):
                hits.append((n, sub["name"].lower()))
            else:
                names = [x for x in (sub["name"].lower(), sub["slug"].replace("-", " ")) if len(x) >= 3]
                order = names + sorted(terms - set(names), key=len, reverse=True)   # specific name before class
                t = next((t for t in order if re.search(r"(?<![a-z0-9])" + re.escape(t) + r"(?![a-z0-9])", low)), None)
                if t: hits.append((n, t))
        for n, t in hits[:2]:
            rows.append({"practice": practice["slug"], "substance": sub["slug"], "matched": t, "section": n["section"],
                         "cue": _cue(n["text"]), "severity": "unrated", "standing": "practice-page prose (not a graded interaction)",
                         "excerpt": n["text"], "source_url": practice["url"]})
    return rows

# psychedex class-rule labels use plural/abbreviated names; substance pages use class names.
ALIASES = {
    "maois": ["monoamine oxidase inhibitor", "monoamine oxidase a inhibitor", "rima", "maoi", "beta-carboline", "β-carboline"],
    "ssris": ["selective serotonin reuptake inhibitor", "ssri"],
    "snris": ["serotonin-norepinephrine reuptake inhibitor", "snri"],
    "nris": ["norepinephrine reuptake inhibitor"],
    "ndris": ["norepinephrine-dopamine reuptake inhibitor", "ndri"],
    "psychedelics": ["psychedelic"], "stimulants": ["stimulant"], "opioids": ["opioid"],
    "benzodiazepines": ["benzodiazepine"], "barbiturates": ["barbiturate"],
    "amphetamines": ["amphetamine", "substituted amphetamine"], "antihistamines": ["antihistamine"],
    "anticholinergics": ["anticholinergic", "deliriant"], "antipsychotics": ["antipsychotic"],
    "atypical antipsychotics": ["atypical antipsychotic"], "dopamine agonists": ["dopamine receptor agonist"],
    "synthetic cannabinoids": ["synthetic cannabinoid"], "cannabis": ["cannabinoid"], "thc": ["cannabinoid"],
    "dissociatives": ["dissociative"], "local anesthetics": ["local anesthetic"], "nsaids": ["nsaid"],
}

def _label_terms(label):
    """'MAOIs (class rule); covers: X, Y' -> normalised candidate terms."""
    head, _, covers = label.partition("; covers:")
    head = re.sub(r"\(class rule\)", "", head)
    terms = set()
    for t in re.split(r"[,()]", head) + covers.split(","):
        t = t.strip().lower()
        if not t: continue
        terms.add(t)
        if t.endswith("s") and len(t) > 4: terms.add(t[:-1])
        terms.update(ALIASES.get(t, []))
    return terms

def interaction(a, b):
    """Most severe rule in either page whose label names the other substance or one of its classes."""
    best = None
    for x, y in ((a, b), (b, a)):
        keys = {y["name"].lower(), y["slug"].lower()} | {c.lower() for c in y["classes"]}
        for r in x["_interactions"]:
            if keys & _label_terms(r["label"]):
                if r["severity"] in SEV_RANK and (best is None or SEV_RANK[r["severity"]] < SEV_RANK[best["severity"]]):
                    best = dict(r, source_url=x["url"])
    if best is None:
        return {"a": a["slug"], "b": b["slug"], "severity": "unknown",
                "note": "unclassified - unknown is never 'safe'", "source_url": None}
    return {"a": a["slug"], "b": b["slug"], "severity": best["severity"], "rule": best["label"],
            "standing": best["standing"], "confidence": best["confidence"], "source_url": best["source_url"]}

# ---------------- blocklist (applied before the toxicity filter) ----------------
BLOCK_RULES = [
    ("opioid", lambda r: any("opioid" in c.lower() for c in r["classes"])),
    ("pcp-type-dissociative", lambda r: r["slug"] in {"pcp", "rolicyclidine", "pce", "pcpy", "tcp", "3-meo-pcp", "3-meo-pce", "4-meo-pcp", "3-ho-pcp", "3-meo-pcpy"}
        or any("arylcyclohexylamine" in c.lower() for c in r["classes"])),
    ("ghb-gbl", lambda r: r["slug"] in {"ghb", "gbl", "1-4-butanediol", "ghv", "gvl"} or any(c.lower() in {"gabapentinoid-ghb", "ghb receptor agonist"} for c in r["classes"])),
    ("nbome", lambda r: "nbome" in r["slug"] or any("nbome" in c.lower() for c in r["classes"])),
]
def blocked(rec):
    return [n for n, f in BLOCK_RULES if f(rec)]

def robots_ok(cache):
    txt = fetch(PDX + "/robots.txt", cache)
    dis = [l.split(":", 1)[1].strip() for l in txt.splitlines() if l.lower().startswith("disallow:")]
    delay = [float(l.split(":", 1)[1]) for l in txt.splitlines() if l.lower().startswith("crawl-delay:")]
    return dis, (max(delay) if delay else 2.0)

def cmd_build_cache(a):
    """One-off polite crawl -> cache of the first N days of picks. Only harness fields + URL."""
    tmp = tempfile.mkdtemp(prefix="pdx-raw-")
    try:
        disallow, delay = robots_ok(tmp)
        delay = max(delay, 2.0)
        global_sleep = delay
        slugs = substance_slugs(tmp)
        seed_hex = a.seed_hex or os.urandom(32).hex()
        order = sorted(slugs, key=lambda s: sha(seed_hex + s))
        pool, excluded, tried = [], [], 0
        for s in order:
            if len(pool) >= a.pool or tried >= a.max_fetch: break
            url = f"{PDX}/substances/{s}.md"
            if any(url.split("psychedex.org", 1)[1].startswith(d) for d in disallow if d): continue
            time.sleep(global_sleep); tried += 1
            try:
                rec = parse_substance(s, tmp)
            except Exception as ex:
                excluded.append({"slug": s, "reasons": [f"fetch-failed: {type(ex).__name__}"], "source_url": url}); continue
            why = []
            if a.filters:   # opt-in only (user decision, iteration 3)
                why = blocked(rec)
                if not why and rec["acute_toxicity"] not in ("Negligible", "Low"):
                    why = [f"acute-toxicity-{rec['acute_toxicity'].lower()}"]
            if why: excluded.append({"slug": s, "name": rec["name"], "reasons": why, "source_url": url}); continue
            pool.append(rec)
        practices = []
        for meta in practice_slugs(tmp):
            url = f"{PDX}/practices/{meta['slug']}"
            if any(url.split("psychedex.org", 1)[1].startswith(d) for d in disallow if d): continue
            time.sleep(global_sleep); tried += 1
            try: practices.append(parse_practice(meta, tmp))
            except Exception as ex: excluded.append({"slug": meta["slug"], "kind": "practice", "reasons": [f"fetch-failed: {type(ex).__name__}"], "source_url": url})
        days = []
        for d in range(a.days):
            n = 1 + u32(seed_hex, 10_000 + d) % 2
            picks = []
            for k in range(n):
                p = pool[u32(seed_hex, 20_000 + d * 8 + k) % len(pool)]["slug"]
                if p not in picks: picks.append(p)
            pr = practices[u32(seed_hex, 30_000 + d) % len(practices)]["slug"] if practices else None
            days.append({"day_index": d, "picks": picks, "practice": pr})
        subs = {r["slug"]: {"slug": r["slug"], "name": r["name"], "url": r["url"], "page_updated": r["page_updated"],
                            "classes": r["classes"], "acute_toxicity": r["acute_toxicity"], "effects": r["effects"],
                            "interactions": r["_interactions"]} for r in pool}
        prs = {p["slug"]: p for p in practices}
        out = {"schema": "psychedex-pick-cache/v2", "filters": {"enabled": bool(a.filters), "blocklist": [n for n, _ in BLOCK_RULES] if a.filters else [], "acute_toxicity_allow": ["Negligible", "Low"] if a.filters else "all",
                   "decision": "User decision (iteration 3): blocklist and toxicity filter removed from selection by default; uncensored documentation of effects. Re-enable with --filters."},
               "practices": prs, "practice_count": len(prs), "built_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
               "seed_hex": seed_hex, "robots_disallow": disallow, "request_delay_s": global_sleep, "pages_fetched": tried,
               "slug_list_size": len(slugs), "pool_size": len(pool), "excluded": excluded, "days": days, "substances": subs,
               "fields_note": "Substances: slug, name, url, page_updated, classes, acute_toxicity, effect names and interaction rows (label, severity, standing, confidence). Practices: slug, name, category, url, effect names, linked substance slugs and dose-free interaction/contraindication sentences. No doses, routes or durations.",
               "source": "https://psychedex.org/llms-full.txt + /substances/<slug>.md + /practices/<slug> (public pages, robots.txt honoured)"}
        out["cache_sha256"] = sha(canon(out))
        json.dump(out, open(a.out, "w"), indent=1, ensure_ascii=False)
        print(json.dumps({k: out[k] for k in ("pages_fetched", "pool_size", "slug_list_size", "practice_count")}), len(excluded), "excluded")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)   # raw pages (with dose tables) are never kept

def from_cache(c, slug):
    r = dict(c["substances"][slug]); r["_interactions"] = r.pop("interactions"); return r

# ---------------- fresh commit-reveal draw (no memory) ----------------
def u32(seed_hex, i): return struct.unpack("<I", hashlib.sha256(bytes.fromhex(seed_hex) + struct.pack("<I", i)).digest()[:4])[0]

def draw(seed_hex, pool, practices):
    """Fresh draw: 1-2 substances from `pool` (already excludes used ones), 1 practice. No memory."""
    want = 1 + u32(seed_hex, 0) % 2
    if len(pool) < want:
        raise SystemExit(f"substance pool exhausted: {len(pool)} left, {want} needed - every substance has appeared once")
    p = sorted(pool)
    for i in range(len(p) - 1, 0, -1):          # Fisher-Yates keyed by the revealed seed
        j = u32(seed_hex, 1000 + i) % (i + 1); p[i], p[j] = p[j], p[i]
    pr = sorted(practices)
    return p[:want], (pr[u32(seed_hex, 7) % len(pr)] if pr else None)

def commit_seed(state, state_dir, day):
    import secrets, datetime
    sd = os.path.join(state_dir, "seeds"); os.makedirs(sd, exist_ok=True)
    f = os.path.join(sd, f"{day}.seed")
    if day in state.setdefault("commits", {}): return state["commits"][day]
    seed = secrets.token_hex(32)
    fd = os.open(f, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600); os.write(fd, seed.encode()); os.close(fd)
    c = {"day": day, "commit_sha256": sha(bytes.fromhex(seed)), "committed_at": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds")}
    state["commits"][day] = c
    return c

def reveal_seed(state, state_dir, day):
    import datetime
    c = state.get("commits", {}).get(day)
    if not c: raise SystemExit(f"no commitment for {day}: run `daily.py commit --day {day}` before the day")
    if c["committed_at"][:10] > day: raise SystemExit(f"commitment for {day} was made after the day started")
    seed = open(os.path.join(state_dir, "seeds", f"{day}.seed")).read().strip()
    if sha(bytes.fromhex(seed)) != c["commit_sha256"]: raise SystemExit("revealed seed does not match the commitment")
    return seed, dict(c, seed_hex=seed, revealed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
                      verify="sha256(bytes.fromhex(seed_hex)) == commit_sha256")

def cmd_commit(a):
    st_dir = os.path.dirname(a.state) or "."
    state = json.load(open(a.state)) if os.path.exists(a.state) else {"day_index": -1, "prev_record_sha256": "0"*64, "used_substances": []}
    c = commit_seed(state, st_dir, a.day); json.dump(state, open(a.state, "w"), indent=1)
    print(json.dumps(c, indent=1))

# ---------------- set/setting submissions ----------------
BLOCK = [r"https?://", r"www\.", r"@[a-z0-9_]{2,}", r"\b[\w.+-]+@[\w-]+\.\w+", r"\+?\d[\d\s().-]{7,}\d",
         r"\b\d+(\.\d+)?\s?(mg|mcg|ug|µg|g|ml|tabs?|hits?|drops?|grams?)\b",
         r"\b(buy|sell|vendor|plug|dealer|darknet|telegram|dose|dosing|redose|mix with)\b"]
def prefilter(text):
    """Deterministic first pass. Passing does NOT approve; a human moderator still reviews."""
    reasons = []
    if not (8 <= len(text) <= 280): reasons.append("M1-length")
    if any(ord(c) < 0x20 or ord(c) > 0x7e for c in text): reasons.append("M2-ascii-only")
    for i, p in enumerate(BLOCK):
        if re.search(p, text, re.I): reasons.append(f"M3-block-{i}")
    return reasons

def take_submission(queue_dir, day):
    if not queue_dir or not os.path.isdir(queue_dir): return None
    subs = []
    for f in sorted(os.listdir(queue_dir)):
        if f.endswith(".json"):
            s = json.load(open(os.path.join(queue_dir, f)))
            if s.get("status") == "approved" and s.get("target_day") == day: subs.append(s)
    subs.sort(key=lambda s: (s["paid_at"], s["id"]))
    return subs[0] if subs else None

# ---------------- prompts ----------------
MOTIF = {"lethal": "the frame is split by a hard crack; draw a warning sign made of ! characters",
         "dangerous": "edges fray into static made of : and . characters",
         "caution": "two patterns overlap and interfere",
         "low risk": "patterns blend softly", "low_risk": "patterns blend softly",
         "unknown": "part of the picture is hidden in fog made of . characters"}

PCUE = {"risk": "the two rhythms grind against each other; jagged edges made of / and \\ characters",
        "attenuates": "one pattern dims the other; half the lines are faint dots",
        "adjunct": "a calm frame holds the scene; a border of = characters",
        "mention": "a thin thread of - characters joins two shapes"}

def symbient_prompt(persona, picks, inters, env, frame_no, practice=None, pinters=()):
    feel = "; ".join(f"{p['name']} ({', '.join(p['effects']) or 'unnamed effects'})" for p in picks)
    mot = "; ".join(MOTIF.get(i["severity"], MOTIF["unknown"]) for i in inters) or "a single clear pattern"
    return (f"You are {persona['name']}, {persona['voice']}\n"
            f"Draw ONE piece of ASCII art. Rules: at most 48 characters per line, at most 24 lines, "
            f"only plain keyboard characters, no words longer than 12 letters, no numbers, no doses, "
            f"no instructions, no real people. Output only the art.\n"
            f"Today, drawn fresh: {feel}.\n"
            + (f"Practice today: {practice['name']} ({practice['category']}; {', '.join(practice['effects']) or 'quiet attention'}). "
               f"Let its rhythm shape the composition.\n" if practice else "")
            + ("Practice and substance meet: " + "; ".join(PCUE.get(r["cue"], PCUE["mention"]) for r in pinters) + ".\n" if pinters else "")
            + f"Visual motif: {mot}.\nSetting: {env or persona['default_setting']}.\nVariation {frame_no + 1}.")

# ---------------- Kara & Gippy (off-chain frame) ----------------
def session_prompt(picks, practice, inters, pinters, env=None):
    subs = " and ".join(p["name"] for p in picks)
    eff = "; ".join(f"{p['name']}: {', '.join(p['effects'][:6]) or 'effects not listed'}" for p in picks)
    risk = "; ".join(f"{i['a']} + {i['b']}: {i['severity']}" for i in inters) or "single substance"
    pr = f"{practice['name']} ({practice['category']})" if practice else "no practice"
    return ("Write a conversation between Kara and Gippy. Every line starts with 'Kara:' or 'Gippy:'.\n"
            "KARA:\n" + primer("kara.md") + "\n\nGIPPY:\n" + primer("gippy.md") + "\n\n"
            "Today Gippy has taken " + subs + " and they practise " + pr + ". Setting: " + (env or "a quiet room with one window at dusk") + ".\n"
            "Known effects: " + eff + ". Recorded interaction labels: " + risk + ".\n"
            "Shape: arrival, rising, peak, coming down. Gippy's perception and beliefs bend with today's draw. "
            "Kara reflects and asks one open question at a time; she never steers, warns or instructs. "
            "Rules: 16 to 28 lines; no amounts, numbers, doses or units; no instructions or advice; no real people; plain English.")

def painting_prompt(summary, hue):
    return ("Kara and Gippy start a fresh session to curate ONE ASCII painting of the session summarised below. "
            "Kara has already drawn the still frame and the horizon. Gippy fills the inside: 10 to 20 lines of motion and "
            "texture made only of symbols such as ~ * o . : / \\ ( ) ^ and spaces, at most 56 characters per line, no words, "
            "no numbers, no border. Then one last line: a short title in plain words. Output only the lines.\nSummary: " + summary)

LABEL = re.compile(r"^\s*[A-Za-z]+\s*\d*\s*[:.)-]\s*")   # model-added row labels
ECHO = "~ * o : / \\ ( ) ^"   # the model sometimes repeats the prompt's symbol list as its "art"

def painting_prompt_alt(summary, hue):
    """Fallback when every attempt only echoed the symbol list: describe the marks in words, list no glyphs."""
    return ("Kara and Gippy start a fresh session to curate ONE ASCII painting of the session summarised below. "
            "Kara has already drawn the still frame and the horizon. Gippy fills the inside with 10 to 20 different lines "
            "of waves, dots, slashes, brackets and spirals that drift and change from line to line, at most 56 characters "
            "per line, no words, no numbers, no border. Then one last line: a short title in plain words. "
            "Output only the lines.\nSummary: " + summary)

KARA_STEERING = re.compile(r"\b(do not|don't|dont|should|must|need to|have to|dangerous|danger|risk|safe|safety|careful|warn|stop)\b", re.I)

def _shingles(t, n=6):
    w = re.findall(r"[a-z']+", t.lower()); return {" ".join(w[i:i + n]) for i in range(len(w) - n + 1)}

def parroted(lines):
    """Lines that copy 6+ consecutive words from the primers (the small model tends to recite them)."""
    ref = _shingles(primer("kara.md") + " " + primer("gippy.md"))
    return sum(1 for x in lines if _shingles(x["text"]) & ref)

def kara_violations(lines):
    """Kara never steers, warns or instructs: count her lines that do."""
    return sum(1 for x in lines if x["speaker"] == "Kara" and KARA_STEERING.search(x["text"]))

def clean_transcript(raw):
    """Accepts 'Kara: text' and also a bare 'Kara:' line followed by the text on the next lines."""
    lines, masked, cur, buf = [], 0, None, []
    def flush():
        nonlocal masked
        if cur and buf:
            t, n = DOSE_RE.subn("[amount removed]", " ".join(buf)); masked += n
            t = "".join(ch for ch in t if 0x20 <= ord(ch) <= 0x7e)[:280]
            lines.append({"speaker": cur, "text": t})
    for l in raw.splitlines():
        l = l.strip().replace("**", "")
        m = re.match(r"^(kara|gippy)\s*[:\-]\s*(.*)$", l, re.I)
        if m:
            flush(); cur, buf = m.group(1).capitalize(), ([m.group(2).strip()] if m.group(2).strip() else [])
        elif l and cur and not re.match(r"^[A-Za-z]+\s*:", l): buf.append(l)
        elif not l: continue
        else: flush(); cur, buf = None, []
    flush()
    return lines[:40], masked

def clean_painting(raw):
    rows = []
    for l in raw.replace("```", "").splitlines():
        rows.append("".join(ch for ch in l.rstrip() if 0x20 <= ord(ch) <= 0x7e)[:64])
    while rows and not rows[0].strip(): rows.pop(0)
    while rows and not rows[-1].strip(): rows.pop()
    rows = [DOSE_RE.sub("", r) for r in rows[:28]]
    return "\n".join(rows)

def kara_frame(inner, title=""):
    """Kara's part of the painting: a still, sparse frame and a horizon. Gippy's texture goes inside."""
    W = 56
    body = [r[:W].ljust(W) for r in inner][:22] or [" " * W]
    hz = max(1, (len(body) * 2) // 3)
    out = ["+" + "=" * (W + 2) + "+"]
    for i, r in enumerate(body):
        out.append("| " + r + " |")
        if i + 1 == hz: out.append("|" + "-" * (W + 2) + "|")
    out.append("+" + "=" * (W + 2) + "+")
    if title: out.append("  " + title[:W])
    return "\n".join(out)

def split_title(p):
    rows = p.split("\n")
    if rows and re.fullmatch(r"[A-Za-z ,.'-]{3,40}", rows[-1].strip() or "-"): return rows[:-1], rows[-1].strip()
    return rows, ""

META = re.compile(r"(kara|gippy)\s*:|think>|summary|painting|frame|horizon|symbols?\b|output", re.I)
TEX_V, TEX_C = "o*.~^", "~:/\\()-'"

def texture(line):
    """Texture pass for word lines: letters become marks (vowels round, consonants strokes); spacing is kept."""
    return "".join(TEX_V[ord(c) % 5] if c.lower() in "aeiou" else TEX_C[ord(c) % 8] if c.isalpha() else ("." if c.isdigit() else c) for c in line)

def symbol_ratio(painting):
    ch = [c for c in painting if not c.isspace()]
    return sum(1 for c in ch if not c.isalnum()) / max(1, len(ch))

def pair_session(seed_hex, picks, practice, inters, pinters, env=None):
    tp = session_prompt(picks, practice, inters, pinters, env)
    best = None
    for attempt in range(2 * MAX_ATTEMPTS):   # keep the run with the fewest Kara steering lines
        traw0, topts0 = ollama(tp, u32(seed_hex, 700 + attempt), temperature=0.85, num_predict=1600)
        l0, m0 = clean_transcript(traw0)
        if len(l0) < 8 or not {"Kara", "Gippy"} <= {x["speaker"] for x in l0}: continue
        v = kara_violations(l0) + parroted(l0)
        if not best or v < best[0]: best = (v, l0, m0, traw0, topts0, attempt)
        if v == 0: break
    if not best: raise SystemExit("kara & gippy transcript: no usable output")
    kv, lines, masked, traw, topts, tatt = best
    summary = " ".join(f"{x['speaker']}: {x['text']}" for x in lines)[:1600]
    hue = gippy_hue(seed_hex)
    best = None
    for pfun, base in ((painting_prompt, 800), (painting_prompt_alt, 900)):
        pp = pfun(summary, hue)
        for attempt in range(2 * MAX_ATTEMPTS):   # prefer symbol-heavy output (the small model likes words)
            praw, popts = ollama(pp, u32(seed_hex, base + attempt), temperature=0.8, num_predict=1200)
            inner, title = split_title(clean_painting(praw))
            # Kara draws the frame; drop echoed prompt lines, the echoed symbol list, and verbatim repeats
            inner = [r for r in inner if r.strip() and not re.fullmatch(r"[\s=+|-]+", r) and not META.search(r) and ECHO not in r]
            inner = [r for i, r in enumerate(inner) if r not in inner[:i]]
            if len(inner) < 4: continue
            score = symbol_ratio("\n".join(inner))
            if not best or score > best[0]: best = (score, inner, title, praw, popts, pp)
            if score >= 0.7: break
        if best: break
    if not best: raise SystemExit("kara & gippy painting: no usable output")
    pp = best[5]; best = best[:5]
    score, inner, title, praw, popts = best
    labelled = [r for r in inner if LABEL.match(r) or re.search(r"[A-Za-z0-9]", r)]
    inner = [LABEL.sub("", r) for r in inner]                     # "WAVE 3: //\/" -> "//\/"
    textured = sum(1 for r in inner if symbol_ratio(r) < 0.5)
    # word lines become marks; stray letters/digits inside symbol lines become marks too (no words, no numbers)
    inner = [texture(r) if symbol_ratio(r) < 0.5 or re.search(r"[A-Za-z0-9]", r) else r for r in inner]
    textured = max(textured, len(labelled))
    if META.search(title): title = ""
    painting = kara_frame(inner, title)
    return {"transcript": lines, "masked_amounts": masked, "kara_steering_lines": kara_violations(lines), "parroted_lines": parroted(lines), "transcript_attempt": tatt, "painting": painting, "painting_symbol_ratio": round(score, 2), "painting_texture_pass_lines": textured,
            "palette": {"kara": KARA_PALETTE, "gippy_hue": hue},
            "transcript_meta": {"prompt_sha256": sha(tp), "raw_sha256": sha(traw), "options": topts},
            "painting_meta": {"prompt_sha256": sha(pp), "raw_sha256": sha(praw), "options": popts, "session": "fresh (only the summary is shared)"},
            "primers_sha256": {n: sha(open(os.path.join(PERSONA_DIR, n), "rb").read()) for n in ("kara.md", "gippy.md", "kara-gippy.md")},
            "prompts": {"transcript": tp, "painting": pp}}

# ---------------- commands ----------------
def cmd_marain(a):
    info = model_info()
    prompt = ("Write 30 short original sayings, one per line, each 3 to 7 words, lowercase, only letters "
              "and spaces. Themes: light, orbits, ships, minds, kindness, distance, play. No names, no quotes "
              "from books, no numbering.")
    seed = int(sha("marain-draft|" + a.batch)[:8], 16)
    raw, opts = ollama(prompt, seed, temperature=0.9, num_predict=600)
    lines, seen = [], set()
    for l in raw.splitlines():
        l = re.sub(r"^[\s\-*\d.)]+", "", l).strip().lower().rstrip(".")
        if re.fullmatch(r"[a-z ]{6,60}", l) and 2 <= len(l.split()) <= 8 and l not in seen:
            seen.add(l); lines.append(l)
    rec = {"batch": a.batch, "status": "UNREVIEWED MODEL DRAFT - curator must approve", "model": dict(info, options=opts),
           "prompt_sha256": sha(prompt), "raw_output_sha256": sha(raw), "prompt": prompt, "raw_output": raw,
           "candidates": lines}
    rec["record_sha256"] = sha(canon(rec))
    json.dump(rec, open(a.out, "w"), indent=1)
    print(len(lines), "candidates ->", a.out)

def cmd_symbient(a):
    persona = json.load(open(a.persona))
    st_dir = os.path.dirname(a.state) or "."
    state = json.load(open(a.state)) if os.path.exists(a.state) else {"day_index": -1, "prev_record_sha256": "0"*64}
    state.setdefault("used_substances", []); state.pop("memory", None); state["persona"] = persona["id"]
    day_index = state["day_index"] + 1
    seed_hex, reveal = reveal_seed(state, st_dir, a.day)
    used = set(state["used_substances"])
    if a.source == "cache":
        c = json.load(open(a.cache))
        pool = [s for s in c["substances"] if s not in used]
        slugs, pslug = draw(seed_hex, pool, list(c.get("practices", {})))
        picks = [from_cache(c, s) for s in slugs]
        practice = c["practices"].get(pslug) if pslug else None
        source = {"kind": "cache", "cache_sha256": c["cache_sha256"]}
    else:
        cache = os.path.join(st_dir, "psychedex-live")
        pool = [s for s in substance_slugs(cache) if s not in used]
        metas = practice_slugs(cache)
        slugs, pmeta = draw(seed_hex, pool, metas)
        picks = [parse_substance(s, cache) for s in slugs]
        practice = parse_practice(pmeta, cache) if pmeta else None
        source = {"kind": "live"}
    remaining = len(pool) - len(picks)
    act = sorted(picks, key=lambda x: x["slug"])
    inters = [interaction(x, y) for i, x in enumerate(act) for y in act[i+1:]]
    worst = min((SEV_RANK.get(i["severity"], 1.5) for i in inters), default=9)
    pinters = practice_interactions(practice, act) if practice else []
    flicker = "1;.35;1;.8;1" if worst <= 1 else ("1;.7;1" if worst <= 2 else None)
    sub = take_submission(a.queue, a.day)
    env = sub["text"] if sub else None
    info = model_info()
    frames = []
    for k in range(a.frames):
        prompt = symbient_prompt(persona, picks, inters, env, k, practice, pinters)
        rejected = []
        for attempt in range(MAX_ATTEMPTS):
            fseed = u32(seed_hex, 100 + k + 1000 * attempt)
            raw, opts = ollama(prompt, fseed)
            with tempfile.NamedTemporaryFile("w", delete=False) as t: t.write(raw)
            ok = subprocess.run([SVGC, "sanitize", t.name], capture_output=True, text=True); os.unlink(t.name)
            if ok.returncode == 0: break
            rejected.append({"seed": fseed, "raw_sha256": sha(raw), "reason": ok.stderr.strip()})
        else:
            raise SystemExit(f"frame {k}: no acceptable output after {MAX_ATTEMPTS} attempts")
        frames.append({"seed": fseed, "options": opts, "prompt_sha256": sha(prompt), "raw_sha256": sha(raw),
                       "rejected_attempts": rejected, "prompt": prompt, "raw": raw})
    ww = pair_session(seed_hex, picks, practice, inters, pinters, env) if not a.no_session else None
    os.makedirs(a.out, exist_ok=True)
    cas = os.path.join(os.path.dirname(a.state) or ".", "cas")
    os.makedirs(cas, exist_ok=True)
    for k, f in enumerate(frames):
        with tempfile.NamedTemporaryFile("w", delete=False) as t: t.write(f["raw"])
        rich = json.loads(subprocess.run([SVGC, "rich-text", t.name], capture_output=True, text=True, check=True).stdout); os.unlink(t.name)
        page = rich_html(persona, a.day, day_index, k, picks, inters, rich["text"], practice, pinters, ww, seed_hex)
        p = os.path.join(cas, "tmp.html"); open(p, "w").write(page)
        cid = subprocess.run([SVGC, "cid", p], capture_output=True, text=True, check=True).stdout.strip()
        os.replace(p, os.path.join(cas, cid))
        f["rich_cid"] = cid; f["rich_masked_dose_tokens"] = rich["masked_dose_tokens"]
    caption = f"{persona['name']} - day {day_index} - " + " + ".join(p["name"] for p in picks) + (f" / {practice['name']}" if practice else "")
    build = {"raw_frames": [f["raw"] for f in frames], "rich_cids": [f["rich_cid"] for f in frames], "caption": caption[:70], "flicker": flicker, "seed_hex": seed_hex}
    json.dump(build, open(os.path.join(a.out, "day-build.json"), "w"), indent=1)
    record = {"schema": "symbient-day-record/v2", "persona": persona["id"], "day": a.day, "day_index": day_index,
              "prev_record_sha256": state["prev_record_sha256"], "day_seed": seed_hex,
              "picks": [{k: v for k, v in p.items() if not k.startswith("_")} for p in picks],
              "practice": {k: practice[k] for k in ("slug", "name", "category", "url", "effects")} if practice else None,
              "practice_interactions": pinters,
              "interactions": inters, "flicker": flicker,
              "draw": dict(reveal, method="commit-reveal; Fisher-Yates over unused substances keyed by sha256(seed||i)", pool_before=len(pool),
                           remaining_after=remaining, used_before=len(used), exhausted_next=remaining < 2),
              "session": ({k: v for k, v in ww.items() if k != "prompts"} | {"model": info, "personas": "Kara & Gippy (original; personas/kara.md, gippy.md, kara-gippy.md)"}) if ww else None,
              "environment": {"submission_id": sub["id"], "text_sha256": sha(sub["text"])} if sub else None,
              "source": source, "model": info, "frames": [{k: v for k, v in f.items() if k not in ("prompt", "raw")} for f in frames],
              "filters": {"enabled": bool(getattr(a, "filters", False))},
              "labels_note": "Interaction labels are AI-assembled data from psychedex.org with their review standing; 'unknown' = unclassified, 'unrated' = prose without a grade."}
    json.dump(record, open(os.path.join(a.out, "day-record.partial.json"), "w"), indent=1)
    json.dump({"prompts": [f["prompt"] for f in frames], "raw": [f["raw"] for f in frames], "session_prompts": ww and ww["prompts"]}, open(os.path.join(a.out, "llm-io.json"), "w"), indent=1)
    state["used_substances"] = sorted(used | {p["slug"] for p in picks})
    state.update({"day_index": day_index, "pending_record": os.path.join(a.out, "day-record.partial.json"),
                  "exhaustion": {"pool_remaining": remaining, "total_used": len(state["used_substances"])}})
    json.dump(state, open(a.state, "w"), indent=1)
    print(json.dumps({"remaining_substances": remaining, "commit": reveal["commit_sha256"], "picks": [p["slug"] for p in picks], "practice": practice and practice["slug"], "interactions": inters, "practice_interactions": pinters, "flicker": flicker}, indent=1))

def rich_html(persona, day, day_index, k, picks, inters, text, practice=None, pinters=(), ww=None, seed_hex=None):
    """Self-contained, script-free page (CSP default-src 'none'); one IPFS block."""
    e = html.escape
    pk = ", ".join(f'<a href="{e(p["url"])}">{e(p["name"])}</a>' for p in picks)
    it = "".join(f"<li>{e(i['a'])} + {e(i['b'])}: <b>{e(i['severity'])}</b>" + (f" ({e(i.get('standing',''))})" if i.get("standing") else "")
                 + (f' <a href="{e(i["source_url"])}">source</a>' if i.get("source_url") else "") + "</li>" for i in inters)
    pr = (f'<p>Practice: <a href="{e(practice["url"])}">{e(practice["name"])}</a> ({e(practice["category"])})</p>' if practice else "")
    pi = "".join(f"<li>{e(r['practice'])} + {e(r['substance'])}: <b>unrated</b>, {e(r['cue'])} ({e(r['standing'])}) - &ldquo;{e(r['excerpt'])}&rdquo; "
                 f'<a href="{e(r["source_url"])}">source</a></li>' for r in pinters)
    wwh, hue = "", gippy_hue(seed_hex) if seed_hex else 160
    if ww:
        k0, k1, k2 = KARA_PALETTE
        wwh = ("<h2>Session: Kara &amp; Gippy</h2><div class=\"t\">" + "".join(f"<p class=\"{e(x['speaker'].lower())}\"><b>{e(x['speaker'])}:</b> {e(x['text'])}</p>" for x in ww["transcript"]) + "</div>"
               "<h2>Painting, curated by Kara &amp; Gippy</h2><pre class=\"p\">" + e(ww["painting"]) + "</pre>"
               "<p class=\"a\">" + CREDIT + " Transcript and painting were written by a local open model. Interaction labels: <a href=\"https://psychedex.org/\">psychedex.org</a>.</p>")
    style_pair = (f".kara b{{color:{KARA_PALETTE[1]}}}.kara{{color:{KARA_PALETTE[2]}}}.gippy b{{color:hsl({hue},80%,68%)}}"
                  f".p{{border:1px solid {KARA_PALETTE[0]};border-left:3px solid hsl({hue},70%,55%);color:hsl({hue},60%,78%)}}")
    return ("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">"
            "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\">"
            f"<title>Latent Benders: A Study - {e(day)} - frame {k + 1}</title>"
            "<style>body{background:#0a0a12;color:#cfe;font:13px/1.35 monospace;margin:2rem;max-width:60rem}pre{white-space:pre;overflow:auto}a{color:#9cf}"
            ".t p{margin:.3rem 0}.p{padding:.8rem}.a{color:#89a;font-size:11.5px}" + style_pair + "</style></head><body>"
            f"<h1>Latent Benders: A Study &middot; {e(day)} &middot; frame {k + 1}</h1><p>Today: {pk}</p>{pr}<ul>{it}{pi}</ul>"
            f"<pre>{e(text)}</pre>{wwh}"
            "</body></html>")

def cmd_resession(a):
    """Off-chain frame only: new session.json + rebuilt rich CAS pages. day-record.json (the on-chain seed) is not changed."""
    persona = json.load(open(a.persona))
    rec = json.load(open(os.path.join(a.out, "day-record.json"))); build = json.load(open(os.path.join(a.out, "day-build.json")))
    seed_hex = rec["draw"]["seed_hex"]; info = model_info()
    ww = pair_session(seed_hex, rec["picks"], rec["practice"], rec["interactions"], rec["practice_interactions"])
    cas = os.path.join(os.path.dirname(a.state) or ".", "cas"); cids = []
    for k, raw in enumerate(build["raw_frames"]):
        with tempfile.NamedTemporaryFile("w", delete=False) as t: t.write(raw)
        rich = json.loads(subprocess.run([SVGC, "rich-text", t.name], capture_output=True, text=True, check=True).stdout); os.unlink(t.name)
        page = rich_html(persona, rec["day"], rec["day_index"], k, rec["picks"], rec["interactions"], rich["text"], rec["practice"], rec["practice_interactions"], ww, seed_hex)
        p = os.path.join(cas, "tmp.html"); open(p, "w").write(page)
        cid = subprocess.run([SVGC, "cid", p], capture_output=True, text=True, check=True).stdout.strip()
        os.replace(p, os.path.join(cas, cid)); cids.append(cid)
    out = {"schema": "latent-benders-session/v1", "day": rec["day"], "record_sha256": rec["record_sha256"], "model": info,
           "rich_cids": cids, "onchain_frame_cids": build["rich_cids"],
           "note": "on-chain templates are immutable: tokens keep linking onchain_frame_cids; rich_cids are the current pages"}
    out.update({k: v for k, v in ww.items() if k != "prompts"})
    json.dump(out, open(os.path.join(a.out, "session.json"), "w"), indent=1)
    json.dump(ww["prompts"], open(os.path.join(a.out, "session-prompts.json"), "w"), indent=1)
    print(json.dumps({"day": rec["day"], "rich_cids": cids, "lines": len(ww["transcript"]), "symbol_ratio": ww["painting_symbol_ratio"]}))

def cmd_finalize(a):
    """After `svgc symbient`: add packed/sanitized hashes, compute record_sha256 (-> collection seed)."""
    rec = json.load(open(os.path.join(a.out, "day-record.partial.json")))
    rep = json.load(open(os.path.join(a.out, "build-report.json")))
    san = json.load(open(os.path.join(a.out, "sanitize-report.json")))
    for f, s in zip(rec["frames"], san): f.update(s)
    rec["packed_sha256"] = rep["packed_sha256"]; rec["packed_bytes"] = rep["packed_bytes"]
    rec["record_sha256"] = sha(canon(rec))
    json.dump(rec, open(os.path.join(a.out, "day-record.json"), "w"), indent=1)
    st = json.load(open(a.state)); st["prev_record_sha256"] = rec["record_sha256"]; st.pop("pending_record", None)
    json.dump(st, open(a.state, "w"), indent=1)
    print("record_sha256 (use as collection seed):", rec["record_sha256"])

if __name__ == "__main__":
    ap = argparse.ArgumentParser(); sp = ap.add_subparsers(dest="cmd", required=True)
    m = sp.add_parser("marain-draft"); m.add_argument("--out", required=True); m.add_argument("--batch", default="b001")
    s = sp.add_parser("symbient-day")
    for k in ("--persona", "--day", "--state", "--out"): s.add_argument(k, required=True)
    s.add_argument("--queue"); s.add_argument("--frames", type=int, default=3)
    s.add_argument("--source", choices=["cache", "live"], default="cache"); s.add_argument("--cache")
    s.add_argument("--filters", action="store_true", help="opt-in: re-enable blocklist + toxicity filter (live source)")
    s.add_argument("--no-session", action="store_true", help="skip the Kara & Gippy transcript + painting")
    rs = sp.add_parser("resession", help="regenerate the Kara & Gippy frame + CAS pages for an existing day (day-record untouched)")
    rs.add_argument("--out", required=True); rs.add_argument("--state", required=True); rs.add_argument("--persona", required=True)
    cm = sp.add_parser("commit"); cm.add_argument("--day", required=True); cm.add_argument("--state", required=True)
    b = sp.add_parser("build-cache"); b.add_argument("--out", required=True); b.add_argument("--days", type=int, default=30)
    b.add_argument("--pool", type=int, default=100000); b.add_argument("--max-fetch", type=int, default=100000); b.add_argument("--seed-hex")
    b.add_argument("--filters", action="store_true", help="opt-in: re-enable blocklist + acute-toxicity filter (default off, user decision)")
    f = sp.add_parser("finalize"); f.add_argument("--out", required=True); f.add_argument("--state", required=True)
    a = ap.parse_args()
    {"commit": cmd_commit, "resession": cmd_resession, "marain-draft": cmd_marain, "symbient-day": cmd_symbient, "finalize": cmd_finalize, "build-cache": cmd_build_cache}[a.cmd](a)
