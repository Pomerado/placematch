// Minimal JS glue for the plain C-ABI placematch WASM (no wasm-bindgen). It runs anywhere
// WebAssembly does and reads nothing but the bytes it is given.
const SEPARATOR = "\u0001";

const NONE = Object.freeze({ outcome: "none", index: null, same: [], reasons: [], notes: [], cross_script: false, prefer_on_read: null, rows: [] });

export async function load(bytes) {
  const t0 = performance.now();
  const mod = await WebAssembly.compile(bytes);
  const t1 = performance.now();
  const { exports: x } = await WebAssembly.instantiate(mod, {});
  const t2 = performance.now();
  x.am_init();
  const t3 = performance.now();
  const enc = new TextEncoder();
  const dec = new TextDecoder();
  const put = (s) => {
    const b = enc.encode(s);
    const p = x.am_alloc(b.length || 1);
    new Uint8Array(x.memory.buffer, p, b.length).set(b);
    return [p, b.length];
  };
  const call = (fn, ...texts) => {
    const args = texts.map((t) => put(t || ""));
    try {
      return fn(...args.flat());
    } finally {
      for (const [p, n] of args) x.am_free(p, n || 1);
    }
  };
  const joined = (texts) => {
    for (const t of texts) {
      if (typeof t !== "string") throw new TypeError("each row must be a string or { text, categorized }");
      if (t.includes(SEPARATOR)) throw new TypeError("row text must not contain U+0001");
    }
    return texts.join(SEPARATOR);
  };
  return {
    timings: { compileMs: t1 - t0, instantiateMs: t2 - t1, initMs: t3 - t2 },
    /** Bytes of WebAssembly memory in use (it grows, never shrinks). */
    memoryBytes() {
      return x.memory.buffer.byteLength;
    },
    pick(query, candidates, country) {
      const r = call(x.am_pick, query, joined(candidates), country);
      return r >= 0 ? { outcome: "match", index: r } : r === -1 ? { outcome: "none", index: null } : { outcome: "ambiguous", index: null };
    },
    compare(a, b, country) {
      return ["different", "same", "unknown"][call(x.am_compare, a, b, country)];
    },
    /**
     * The outcome with the rows that are the caller's place, each row's verdict and reasons, why an
     * ambiguous answer asks, notes beside a match, and the row a read may take (`prefer_on_read`).
     * `rows`: strings, or `{ text, categorized }` where `categorized` says the site shows the row
     * with a category of its own (a venue type), as it does not for a dropped pin.
     */
    pickExplained(query, rows, options = {}) {
      if (rows.length === 0) return { ...NONE, same: [], reasons: [], notes: [], rows: [] };
      const texts = rows.map((r) => (typeof r === "string" ? r : r?.text));
      const flags = rows.map((r) => (typeof r === "object" && r?.categorized ? "1" : "0")).join("");
      const p = call(x.am_pick_explained, query, joined(texts), options.country, flags);
      return JSON.parse(dec.decode(new Uint8Array(x.memory.buffer, p, x.am_out_len())));
    },
  };
}
