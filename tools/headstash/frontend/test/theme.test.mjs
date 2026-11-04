import test from "node:test";
import assert from "node:assert/strict";
import * as V from "../v2lib.js";

test("theme: apricot glow + builtin validate", () => {
  for (const t of [V.APRICOT_GLOW, V.BUILTIN_THEME]) { const v = V.validateTheme(t); assert.deepEqual(Object.keys(v.light).sort(), [...V.THEME_TOKENS].sort()); assert.deepEqual(Object.keys(v.dark).sort(), [...V.THEME_TOKENS].sort()); }
  assert.equal(V.validateTheme({ light: { bg: "#fff" } }).light.bg, "#fff");
  assert.equal(V.validateTheme({ dark: { accent: "#E8B773" }, radius_px: 0 }).radius_px, 0);
});
const BAD = [
  null, [], "x", {}, { light: {} }, { light: { bg: "red" } }, { light: { bg: "red;}body{" } }, { light: { bg: "#fff;}body{background:url(x)}" } },
  { light: { bg: "url(https://evil/x.png)" } }, { light: { bg: "expression(alert(1))" } }, { light: { bg: "#" + "a".repeat(300) } },
  { light: { bg: "#ffff" } }, { light: { bg: "#12345g" } }, { light: { bg: " #ffffff" } }, { light: { bg: "#ffffff " } }, { light: { bg: "#fff\n" } },
  { light: { bg: 0xffffff } }, { light: { "bg;}x{": "#fff" } }, { light: { color: "#fff" } }, { light: { __proto__: null, bg: "#fff" } },
  { light: { bg: "#fff" }, font: "x" }, { light: { bg: "#fff" }, radius_px: 25 }, { light: { bg: "#fff" }, radius_px: -1 }, { light: { bg: "#fff" }, radius_px: 1.5 },
  { light: { bg: "#fff" }, radius_px: "4px;}" }, { light: "x" }, { light: { bg: "#fff" }, dark: [] }, { light: { bg: "var(--x)" } }, { light: { bg: "#fff/**/" } },
];
test("theme: malicious / malformed rejected", () => {
  for (const b of BAD) assert.throws(() => V.validateTheme(b), undefined, JSON.stringify(b));
  assert.throws(() => V.themeCss({ light: { bg: "red;}body{" } }));
});
test("theme: css is only whitelisted --hs-* declarations", () => {
  const css = V.themeCss(V.APRICOT_GLOW);
  assert.match(css, /^:root\[data-theme=light\]\{[^{}]+\}\n:root\[data-theme=dark\]\{[^{}]+\}\n$/);
  for (const d of css.match(/\{([^}]*)\}/g).flatMap((b) => b.slice(1, -1).split(";"))) assert.match(d, /^--hs-(bg|surface|surfaceAlt|text|textMuted|border|borderStrong|accent|onAccent|focus):#[0-9A-Fa-f]{6}$|^--hs-radius:\d{1,2}px$/);
  assert.ok(css.includes("--hs-bg:#FEF4E7") && css.includes("--hs-bg:#170D02"));
});
test("theme: fallback order headstash > manifest > builtin, per-token merge", () => {
  const hs = { light: { accent: "#0055AA" } };
  let p = V.pickTheme([["headstash", hs], ["manifest", V.APRICOT_GLOW]]); assert.equal(p.source, "headstash"); assert.equal(p.theme.light.accent, "#0055AA"); assert.equal(p.theme.light.bg, V.BUILTIN_THEME.light.bg);
  p = V.pickTheme([["headstash", null], ["manifest", V.APRICOT_GLOW]]); assert.equal(p.source, "manifest"); assert.equal(p.theme.dark.bg, "#170D02");
  p = V.pickTheme([["headstash", { light: { bg: "red;}body{" } }], ["manifest", V.APRICOT_GLOW]]); assert.equal(p.source, "manifest"); assert.equal(p.rejected.length, 1);
  p = V.pickTheme([["headstash", undefined], ["manifest", undefined]]); assert.equal(p.source, "builtin");
});
test("theme: mode from storage else prefers-color-scheme", () => {
  const s = new V.MemStorage(); assert.equal(V.initialMode(s, true), "dark"); assert.equal(V.initialMode(s, false), "light");
  s.setItem(V.THEME_MODE_KEY, "light"); assert.equal(V.initialMode(s, true), "light"); s.setItem(V.THEME_MODE_KEY, "x;}"); assert.equal(V.initialMode(s, false), "light");
});
test("theme: WCAG contrast (AA text 4.5, non-text UI 3.0) in both modes", () => {
  assert.ok(Math.abs(V.contrast("#000000", "#FFFFFF") - 21) < 1e-9);
  for (const [name, t] of [["apricot", V.APRICOT_GLOW], ["builtin", V.BUILTIN_THEME]]) for (const m of V.THEME_MODES) {
    const c = t[m], r = (a, b) => V.contrast(c[a], c[b]);
    for (const bg of ["bg", "surface", "surfaceAlt"]) { assert.ok(r("text", bg) >= 4.5, `${name}.${m} text/${bg} ${r("text", bg)}`); assert.ok(r("textMuted", bg) >= 4.5, `${name}.${m} textMuted/${bg} ${r("textMuted", bg)}`); }
    assert.ok(r("onAccent", "accent") >= 4.5, `${name}.${m} onAccent/accent ${r("onAccent", "accent")}`);
    for (const bg of ["bg", "surface"]) { assert.ok(r("accent", bg) >= 3, `${name}.${m} accent/${bg}`); assert.ok(r("focus", bg) >= 3, `${name}.${m} focus/${bg}`); }
  }
});
