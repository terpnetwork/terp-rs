"""python3 -m unittest harness/test_daily.py  (offline; fixtures mirror real psychedex rows)."""
import json, os, sys, unittest
sys.path.insert(0, os.path.dirname(__file__))
import daily

def rec(slug, name, classes, rows=()):
    return {"slug": slug, "name": name, "url": f"https://psychedex.org/substances/{slug}.md", "classes": classes,
            "_interactions": [{"label": l, "severity": s, "standing": "AI-ASSEMBLED", "confidence": "high"} for l, s in rows]}

HARMALINE = rec("harmaline", "Harmaline", ["Indole alkaloid", "Monoamine oxidase inhibitor", "RIMA", "Psychedelic"])
DET = rec("det", "DET", ["Substituted tryptamine", "Tryptamine", "Serotonin 2A receptor agonist", "Psychedelic"],
          [("MAOIs (class rule)", "lethal"), ("SSRIs (class rule)", "dangerous")])

class Interactions(unittest.TestCase):
    def test_maoi_plural_label_matches_class_name(self):
        i = daily.interaction(DET, HARMALINE)          # was 'unknown' in iteration 1
        self.assertEqual(i["severity"], "lethal")
        self.assertEqual(i["rule"], "MAOIs (class rule)")
        self.assertEqual(daily.interaction(HARMALINE, DET)["severity"], "lethal")
    def test_ssri_alias(self):
        sert = rec("sertraline", "Sertraline", ["Selective serotonin reuptake inhibitor"])
        self.assertEqual(daily.interaction(DET, sert)["severity"], "dangerous")
    def test_covers_list(self):
        lsd = rec("lsd", "LSD", ["Psychedelic"], [("Ketamine, DXM, PCP (class rule); covers: NMDA receptor antagonist", "dangerous")])
        k = rec("ketamine", "Ketamine", ["NMDA receptor antagonist"])
        self.assertEqual(daily.interaction(lsd, k)["severity"], "dangerous")
    def test_no_match_is_unknown_not_safe(self):
        i = daily.interaction(rec("a", "A", ["Foo"]), rec("b", "B", ["Bar"]))
        self.assertEqual(i["severity"], "unknown"); self.assertIn("never", i["note"])

class Blocklist(unittest.TestCase):
    def test_blocks(self):
        cases = {"rolicyclidine": ["Arylcyclohexylamine", "Dissociative"], "pcp": ["Dissociative"], "heroin": ["Opioid"],
                 "ghb": ["Depressant"], "gbl": ["Depressant"], "25i-nbome": ["Substituted phenethylamine"],
                 "x": ["μ-opioid receptor agonist"]}
        for slug, cls in cases.items():
            self.assertTrue(daily.blocked(rec(slug, slug, cls)), slug)
        self.assertFalse(daily.blocked(rec("lsd", "LSD", ["Psychedelic", "Lysergamide"])))

PRACTICE_HTML = """<h1>Holotropic breathwork</h1><h2>Subjective Effects</h2><p><a href="/effects/tetany">Breathwork tetany</a></p>
<h3>Substance Interactions</h3><p>Sympathomimetic drugs \u2014 prescribed stimulants, <a href="/substances/cocaine">cocaine</a>, and methamphetamine \u2014
compound the sympathetic load and elevate cardiovascular risk.<a class="cite">[2]</a> Serotonergic antidepressants may attenuate the state.
Some people take 100 mg of something first.</p><h3>Contraindications</h3><p>Pregnancy.</p><h2>Progression</h2>"""

class Practices(unittest.TestCase):
    def setUp(self):
        import tempfile
        self.d = tempfile.mkdtemp()
        open(os.path.join(self.d, "psychedex.org_practices_holotropic-breathwork"), "w").write(PRACTICE_HTML)
        self.p = daily.parse_practice({"slug": "holotropic-breathwork", "name": "Holotropic breathwork", "category": "breathwork"}, self.d)
    def test_parse_keeps_prose_drops_dose_sentences(self):
        self.assertEqual(self.p["effects"], ["Breathwork tetany"])
        self.assertEqual(self.p["linked_substances"], ["cocaine"])
        txt = " ".join(n["text"] for n in self.p["notes"])
        self.assertIn("cardiovascular risk", txt); self.assertNotIn("100 mg", txt); self.assertNotIn("[2]", txt)
    def test_interaction_by_name_is_unrated_risk(self):
        r = daily.practice_interactions(self.p, [rec("cocaine", "Cocaine", ["Stimulant"])])
        self.assertTrue(r); self.assertEqual(r[0]["matched"], "cocaine"); self.assertEqual(r[0]["cue"], "risk")
        self.assertEqual(r[0]["severity"], "unrated"); self.assertEqual(r[0]["source_url"], self.p["url"])
    def test_class_match_and_no_match(self):
        r = daily.practice_interactions(self.p, [rec("amph", "Amphetamine", ["Stimulant"])])
        self.assertEqual(r[0]["matched"], "stimulants")
        self.assertEqual(daily.practice_interactions(self.p, [rec("lsd", "LSD", ["Lysergamide"])]), [])

class FiltersDefaultOff(unittest.TestCase):
    def test_flags_default_off(self):
        import subprocess
        out = subprocess.run([sys.executable, os.path.join(os.path.dirname(__file__), "daily.py"), "build-cache", "-h"], capture_output=True, text=True).stdout
        self.assertIn("--filters", out); self.assertIn("default off", out)

class Cache(unittest.TestCase):
    def test_cache_fields_and_no_doses(self):
        p = os.environ.get("PDX_CACHE")
        if not p: self.skipTest("set PDX_CACHE")
        c = json.load(open(p))
        self.assertEqual(c["schema"], "psychedex-pick-cache/v2")
        self.assertEqual(len(c["days"]), 30)
        self.assertFalse(c["filters"]["enabled"])                       # user decision: unfiltered by default
        allowed = {"slug", "name", "url", "page_updated", "classes", "acute_toxicity", "effects", "interactions"}
        for s in c["substances"].values():
            self.assertEqual(set(s), allowed)
            for r in s["interactions"]: self.assertEqual(set(r), {"label", "severity", "standing", "confidence"})
        self.assertGreater(len(c["practices"]), 0)
        for pr in c["practices"].values():
            self.assertEqual(set(pr), {"slug", "name", "category", "url", "effects", "linked_substances", "notes"})
        for d in c["days"]:
            for slug in d["picks"]: self.assertIn(slug, c["substances"])
            self.assertIn(d["practice"], c["practices"])
        self.assertNotRegex(json.dumps(c, ensure_ascii=False).lower(), r"\b\d+(\.\d+)?\s?(mg|mcg|µg|ug)\b")


class FreshDraw(unittest.TestCase):
    def test_no_repeat_and_exhaustion(self):
        pool, used, seen = [f"s{i}" for i in range(7)], set(), []
        for d in range(10):
            seed = daily.sha(f"day{d}")
            try: picks, pr = daily.draw(seed, [s for s in pool if s not in used], ["p1", "p2"])
            except SystemExit as e: self.assertIn("exhausted", str(e)); break
            self.assertTrue(set(picks).isdisjoint(used)); used |= set(picks); seen.append(picks); self.assertIn(pr, ["p1", "p2"])
        else: self.fail("never exhausted")
        self.assertGreaterEqual(len(used), 6)
    def test_draw_is_keyed_by_seed_only(self):
        a = daily.draw(daily.sha("x"), ["a", "b", "c", "d"], ["p"]); b = daily.draw(daily.sha("x"), ["d", "c", "b", "a"], ["p"])
        self.assertEqual(a, b)

class CommitReveal(unittest.TestCase):
    def test_commit_then_reveal_verifies(self):
        import tempfile
        d = tempfile.mkdtemp(); st = {"day_index": -1}
        c = daily.commit_seed(st, d, "2026-10-20")
        self.assertEqual(daily.commit_seed(st, d, "2026-10-20"), c)      # idempotent, never re-rolled
        seed, rec = daily.reveal_seed(st, d, "2026-10-20")
        self.assertEqual(daily.sha(bytes.fromhex(seed)), c["commit_sha256"]); self.assertEqual(rec["seed_hex"], seed)
        open(os.path.join(d, "seeds", "2026-10-20.seed"), "w").write("00" * 32)
        with self.assertRaises(SystemExit): daily.reveal_seed(st, d, "2026-10-20")
        with self.assertRaises(SystemExit): daily.reveal_seed(st, d, "2026-10-21")

class NoDisclaimerInArt(unittest.TestCase):
    def test_prompt_and_page_carry_no_disclaimer_or_memory(self):
        persona = {"name": "Latent Benders", "voice": "v", "default_setting": "room"}
        pk = [dict(DET, effects=["visuals"])]
        p = daily.symbient_prompt(persona, pk, [], None, 0)
        for bad in ("ictional", "not dosing", "advice", "Echoes", "fading"): self.assertNotIn(bad, p)
        ww = {"transcript": [{"speaker": "Kara", "text": "what is that like?"}, {"speaker": "Gippy", "text": "the floor hums"}], "painting": "=~~=\n-**-"}
        page = daily.rich_html(persona, "2026-10-20", 0, 0, pk, [], "art", None, (), ww, daily.sha("x"))
        self.assertNotIn("ictional", page); self.assertNotIn("Not dosing", page)
        for gone in ("Wib", "Wob", "brain.wibandwob", "CC BY"): self.assertNotIn(gone, page)
        self.assertIn("inspired by wibandwob", page); self.assertIn("Kara &amp; Gippy", page); self.assertIn("Latent Benders: A Study", page)
        self.assertIn("hsl(%d," % daily.gippy_hue(daily.sha("x")), page)
    def test_transcript_masks_amounts(self):
        lines, n = daily.clean_transcript("Gippy: we took 20 mg and waited\nnoise\nKara: what do you notice?\n**Gippy:** fine\nWib: ignored")
        self.assertEqual([l["speaker"] for l in lines], ["Gippy", "Kara", "Gippy"]); self.assertEqual(n, 1)
        self.assertNotIn("mg", lines[0]["text"])


class Personas(unittest.TestCase):
    def test_primers_are_original_and_prompt_safe(self):
        for n in ("kara.md", "gippy.md", "kara-gippy.md"):
            t = open(os.path.join(daily.PERSONA_DIR, n)).read()
            for bad in ("Wib", "Wob", "ictional", "not dosing", " mg", "dose "): self.assertNotIn(bad, t)
        k = daily.primer("kara.md"); g = daily.primer("gippy.md")
        self.assertIn("open questions", k); self.assertNotIn("#5f6b66", k)      # visual section stays out of prompts
        self.assertIn("draw", g)
        p = daily.session_prompt([dict(DET, effects=["visuals"])], None, [], [])
        self.assertIn("Kara:", p); self.assertIn("Gippy", p); self.assertNotIn("Wib", p); self.assertNotIn("ictional", p)
    def test_gippy_hue_follows_seed_kara_fixed(self):
        self.assertNotEqual(daily.gippy_hue("0000" + "0" * 60), daily.gippy_hue("ffff" + "0" * 60))
        self.assertEqual(daily.KARA_PALETTE, ["#5f6b66", "#8a9a8c", "#d8d2c4"])
    def test_bare_speaker_lines_and_kara_steering(self):
        lines, _ = daily.clean_transcript("Kara:\nWhat do you see?\n\nGippy:\nThe floor breathes.\nIt likes me.\nKara: Do not move, it is dangerous.")
        self.assertEqual([x["speaker"] for x in lines], ["Kara", "Gippy", "Kara"]); self.assertEqual(lines[1]["text"], "The floor breathes. It likes me.")
        self.assertEqual(daily.kara_violations(lines), 1)
        self.assertNotIn("slow ocean", daily.primer("gippy.md"))
    def test_kara_frame_wraps_gippy_texture(self):
        inner, title = daily.split_title("~*~ o ~\n:: // ::\n^^ ( ) ^^\nThe Floor Breathes")
        self.assertEqual(title, "The Floor Breathes")
        p = daily.kara_frame(inner, title); rows = p.split("\n")
        self.assertTrue(rows[0].startswith("+=") and rows[-2].startswith("+=")); self.assertTrue(any(r.startswith("|--") for r in rows))
        self.assertTrue(all(len(r) <= 64 for r in rows))
    def test_parrot_and_texture(self):
        k = daily.primer("kara.md").split("\n")[1]
        self.assertEqual(daily.parroted([{"speaker": "Kara", "text": k}, {"speaker": "Gippy", "text": "lights fold"}]), 1)
        t = daily.texture("the floor hums")
        self.assertEqual(len(t), len("the floor hums")); self.assertFalse(any(c.isalnum() for c in t)); self.assertIn(" ", t)
        self.assertTrue(daily.META.search("Kara: hello")); self.assertTrue(daily.META.search("</think>"))
        self.assertIn(daily.ECHO, daily.painting_prompt("x", 0).replace(" .", ""))   # the echo filter matches the prompt's list as the model repeats it
        self.assertNotIn("~", daily.painting_prompt_alt("x", 0))
        self.assertEqual(daily.LABEL.sub("", "WAVE 13: // / ( )"), "// / ( )")                     # the fallback lists no glyphs
    def test_symbol_ratio_prefers_art(self):
        self.assertGreater(daily.symbol_ratio("=~*~=\\n|o.o|"), daily.symbol_ratio("WORDS ONLY HERE"))

if __name__ == "__main__":
    unittest.main()
