import fs from 'fs';
import { packTemplate, unpackTemplate, toLimbs } from './pack.js';
const L = process.env.LP || (process.env.HOME + '/abstract/terp-core/websites/terp.network/lib/svg-launchpad');
const eng = await import(L + '/engine.js');
const dir = '../corpus'; const rows = []; let fail = 0;
for (const f of fs.readdirSync(dir).sort()) {
  const c = JSON.parse(fs.readFileSync(dir + '/' + f));
  const vars = c.variables || eng.variablesFromDictionary(c.dictionary).variables || eng.variablesFromDictionary(c.dictionary);
  const v = Array.isArray(vars) ? vars : vars.variables;
  let packed;
  try { packed = packTemplate(c.svg_template, v); } catch (e) { rows.push({ f, err: e.message }); continue; }
  const back = unpackTemplate(packed);
  const okT = back.svg_template === c.svg_template;
  const okV = JSON.stringify(back.variables) === JSON.stringify(v);
  const sl = eng.computeSlots(back.svg_template, back.variables).slots;
  const okS = !c.template_slots || JSON.stringify(sl.map(s => ({ start: s.start, end: s.end, var_idx: s.var_idx ?? s.varIdx }))) === JSON.stringify(c.template_slots);
  if (!(okT && okV && okS)) fail++;
  const jsonBytes = Buffer.byteLength(JSON.stringify({ svg_template: c.svg_template, variables: v, template_slots: c.template_slots || sl.map(s => ({ start: s.start, end: s.end, var_idx: s.var_idx ?? s.varIdx })) }));
  fs.mkdirSync('out', { recursive: true }); fs.writeFileSync('out/' + f.replace('.json', '.svp'), packed);
  rows.push({ f: c.name, tpl_bytes: Buffer.byteLength(c.svg_template), json_ext_bytes: jsonBytes, packed_bytes: packed.length, packed_b64: Buffer.from(packed).toString('base64').length, limbs_json: JSON.stringify(toLimbs(packed)).length, template_eq: okT, vars_eq: okV, slots_eq: okS });
}
console.table(rows); fs.writeFileSync('roundtrip-js.json', JSON.stringify(rows, null, 1));
if (fail) { console.error('FAIL', fail); process.exit(1); }
