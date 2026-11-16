// Minimal JS glue for the plain C-ABI placematch WASM (no wasm-bindgen).
export async function load(bytes) {
  const t0 = performance.now();
  const mod = await WebAssembly.compile(bytes);
  const t1 = performance.now();
  const { exports: x } = await WebAssembly.instantiate(mod, {});
  const t2 = performance.now();
  x.am_init();
  const t3 = performance.now();
  const enc = new TextEncoder();
  const put = (s) => {
    const b = enc.encode(s);
    const p = x.am_alloc(b.length || 1);
    new Uint8Array(x.memory.buffer, p, b.length).set(b);
    return [p, b.length];
  };
  const call = (fn, a, b, cc) => {
    const [ap, al] = put(a), [bp, bl] = put(b), [cp, cl] = put(cc || "");
    const r = fn(ap, al, bp, bl, cp, cl);
    x.am_free(ap, al || 1); x.am_free(bp, bl || 1); x.am_free(cp, cl || 1);
    return r;
  };
  return {
    timings: { compileMs: t1 - t0, instantiateMs: t2 - t1, initMs: t3 - t2 },
    pick(query, candidates, country) {
      const r = call(x.am_pick, query, candidates.join("\u0001"), country);
      return r >= 0 ? { outcome: "match", index: r } : r === -1 ? { outcome: "none", index: null } : { outcome: "ambiguous", index: null };
    },
    compare(a, b, country) {
      return ["different", "same", "unknown"][call(x.am_compare, a, b, country)];
    },
  };
}
