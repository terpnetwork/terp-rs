// DOMPurify (SVG profile + SMIL) over rendered logo SVGs; reports every stripped node/attr.
//   cd scripts/cw-infuser/sanitize && npm ci && node sanitize-svgs.mjs [dir ...]
// Default dir: ../gas-bench/out/rendered (written by svg-gas-bench). Exit 1 if anything is stripped.
import { JSDOM } from "jsdom";
import createDOMPurify from "dompurify";
import fs from "node:fs";
import path from "node:path";

export const SMIL_TAGS = ["animate", "animateTransform", "animateMotion", "set", "mpath"];
export const SMIL_ATTRS = ["attributeName", "attributeType", "values", "from", "to", "by", "dur", "begin",
  "end", "repeatCount", "repeatDur", "keyTimes", "keySplines", "keyPoints", "calcMode", "additive",
  "accumulate", "restart", "min", "max", "path", "rotate", "type", "fill", "href", "xlink:href"];
export const CONFIG = { USE_PROFILES: { svg: true, svgFilters: true }, ADD_TAGS: SMIL_TAGS, ADD_ATTR: SMIL_ATTRS };

const window = new JSDOM("").window;
const DOMPurify = createDOMPurify(window);
const dirs = process.argv.slice(2).length ? process.argv.slice(2) : [new URL("../gas-bench/out/rendered", import.meta.url).pathname];
let bad = 0, n = 0;
for (const d of dirs) for (const f of fs.readdirSync(d).filter((x) => x.endsWith(".svg")).sort()) {
  const src = fs.readFileSync(path.join(d, f), "utf8");
  DOMPurify.sanitize(src, CONFIG);
  const removed = DOMPurify.removed
    // jsdom wrapper <body> and XML comments are not content loss
    .filter((r) => !(r.element && ["BODY", "#comment"].includes(r.element.nodeName)))
    .map((r) => r.attribute
    ? `@${r.attribute.name} on <${r.from?.nodeName}>` : `<${r.element?.nodeName}>`);
  const smil = (src.match(/<(animate|animateTransform|animateMotion|set)\b/g) || []).length;
  n++;
  if (removed.length) { bad++; console.log(`STRIPPED ${f} (smil=${smil}): ${[...new Set(removed)].join(", ")}`); }
  else console.log(`ok       ${f} (smil=${smil})`);
}
console.log(`${n} files, ${bad} with stripped content`);
process.exit(bad ? 1 : 0);
