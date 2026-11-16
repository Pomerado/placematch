// node bench.mjs <wasm> <cases.jsonl> <predictions-out.jsonl>
import { readFileSync, writeFileSync } from "node:fs";
import { load } from "./placematch.mjs";
const [wasmPath, casesPath, outPath] = process.argv.slice(2);
const bytes = readFileSync(wasmPath);
const tStart = performance.now();
const m = await load(bytes);
const cold = performance.now() - tStart;
const cases = readFileSync(casesPath, "utf8").split("\n").filter(Boolean).map((l) => JSON.parse(l));
const out = [];
const pickT = [], cmpT = [];
for (const c of cases) {
  let t = performance.now();
  let p;
  if (c.task === "pair") {
    const v = m.compare(c.query, c.candidates[0], c.country);
    p = v === "same" ? { outcome: "match", index: 0 } : { outcome: "none", index: null };
    cmpT.push(performance.now() - t);
  } else {
    p = m.pick(c.query, c.candidates, c.country);
    pickT.push(performance.now() - t);
  }
  out.push(JSON.stringify({ id: c.id, ...p }));
}
writeFileSync(outPath, out.join("\n") + "\n");
const q = (a, f) => { const s = [...a].sort((x, y) => x - y); return s[Math.min(s.length - 1, Math.round((s.length - 1) * f))]; };
console.log(JSON.stringify({
  wasmBytes: bytes.length, coldTotalMs: +cold.toFixed(2), ...Object.fromEntries(Object.entries(m.timings).map(([k, v]) => [k, +v.toFixed(2)])),
  pickP50us: +(q(pickT, 0.5) * 1000).toFixed(1), pickP99us: +(q(pickT, 0.99) * 1000).toFixed(1),
  compareP50us: +(q(cmpT, 0.5) * 1000).toFixed(1), compareP99us: +(q(cmpT, 0.99) * 1000).toFixed(1),
  rssMB: +(process.memoryUsage().rss / 1048576).toFixed(1),
}));
