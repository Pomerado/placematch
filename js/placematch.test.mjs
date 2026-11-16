// node --test js/placematch.test.mjs, after tools/build-js.sh put js/placematch.wasm in place.
// Addresses are made up.
import { test } from "node:test";
import assert from "node:assert/strict";
import { open } from "./index.mjs";

const m = await open();

test("pick answers as before", () => {
  const rows = ["12 Maple Avenue, Apt 4, Springfield, IL 62704", "120 Maple Avenue, Springfield, IL 62704"];
  assert.deepEqual(m.pick("12 Maple Ave Apt 4, Springfield IL", rows, "US"), { outcome: "match", index: 0 });
  assert.equal(m.compare("10 Main Street, Springfield", "10 Main St, Springfield, IL", "US"), "same");
});

test("pickExplained lists the place's rows and notes a unit the listing leaves out", () => {
  const e = m.pickExplained("10 Example St, Apt 4, Springfield, IL", ["Example Hotel, 10 Example St, Springfield, IL 62701", "10 Example St, Springfield, IL 62701", "11 Example St, Springfield, IL"], { country: "US" });
  assert.equal(e.outcome, "match");
  assert.equal(e.index, 1);
  assert.deepEqual(e.same, [0, 1]);
  assert.deepEqual(e.notes, ["unit_not_listed"]);
  assert.equal(e.cross_script, false);
  assert.equal(e.rows.length, 3);
  assert.equal(e.rows[2].verdict, "different");
});

test("a same-name pin beside a categorized venue asks; a read may take the venue", () => {
  const e = m.pickExplained("example center", ["Example Center, Othertown, NY, USA", { text: "Jane Q. Example Convention Center, 429 Sample Ave, Exampleton, NY 10001, USA", categorized: true }], { country: "US" });
  assert.equal(e.outcome, "ambiguous");
  assert.deepEqual(e.reasons, ["place_name_variant"]);
  assert.equal(e.prefer_on_read, 1);
});

test("near rows ask with their reason", () => {
  const q = "12 Example St, Springfield, IL 62701";
  assert.equal(m.pickExplained(q, ["10-14 Example St, Springfield, IL 62701"], { country: "US" }).outcome, "match");
  assert.deepEqual(m.pickExplained(q, ["12 Example St, Springfield, IL 62702"], { country: "US" }).reasons, ["postcode_differs"]);
  assert.deepEqual(m.pickExplained("12 Example St, Springfield", ["10-14 Example St, Springfield"]).reasons, ["number_range"]);
  assert.deepEqual(m.pickExplained("12 Example Square, Springfield", ["Example Square, Springfield"]).reasons, ["number_not_listed"]);
});

test("no rows is none, and a row holding the separator is refused", () => {
  assert.deepEqual(m.pickExplained("10 Example St", []), { outcome: "none", index: null, same: [], reasons: [], notes: [], cross_script: false, prefer_on_read: null, rows: [] });
  assert.throws(() => m.pickExplained("10 Example St", ["a\u0001b"]), TypeError);
});

test("many calls keep memory flat", () => {
  const rows = Array.from({ length: 20 }, (_, i) => `${i + 1} Example St, Springfield, IL 62701`);
  m.pickExplained("10 Example St, Springfield, IL 62701", rows, { country: "US" });
  const before = m.memoryBytes();
  for (let i = 0; i < 2000; i++) m.pickExplained("10 Example St, Springfield, IL 62701", rows, { country: "US" });
  assert.equal(m.memoryBytes(), before);
});
