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
  assert.deepEqual(e.rows[0].notes, ["unit_not_listed"]);
  assert.equal(e.cross_script, false);
  assert.equal(e.rows.length, 3);
  assert.equal(e.rows[2].verdict, "different");
});

test("a same-name pin beside a categorized venue asks; a read may take the venue", () => {
  const rows = ["Example Center, Exampleton, NY, USA", { text: "Jane Q. Example Convention Center, 429 Sample Ave, Exampleton, NY 10001, USA", categorized: true }];
  const e = m.pickExplained("example center, exampleton, ny", rows, { country: "US" });
  assert.equal(e.outcome, "ambiguous");
  assert.deepEqual(e.reasons, ["place_name_variant"]);
  assert.equal(e.prefer_on_read, 1);
  // with no area from the caller, a read asks too
  assert.equal(m.pickExplained("example center", rows, { country: "US" }).prefer_on_read, null);
});

test("near rows ask with their reason", () => {
  const q = "12 Example St, Springfield, IL 62701";
  assert.equal(m.pickExplained(q, ["10-14 Example St, Springfield, IL 62701"], { country: "US" }).outcome, "match");
  assert.deepEqual(m.pickExplained(q, ["12 Example St, Springfield, IL 62702"], { country: "US" }).reasons, ["postcode_differs"]);
  assert.deepEqual(m.pickExplained("12 Example St, Springfield", ["10-14 Example St, Springfield"], { country: "US" }).reasons, ["number_range"]);
  // no country given or shown by the texts, or one where a hyphen joins a unit and its number:
  // never a range
  assert.equal(m.pickExplained("12 Example St, Springfield", ["10-14 Example St, Springfield"]).outcome, "none");
  assert.equal(m.pickExplained("5 Example St, Exampleton NSW 2000", ["3-10 Example St, Exampleton NSW 2000"], { country: "AU" }).outcome, "none");
  assert.deepEqual(m.pickExplained("12 Example Square, Springfield", ["Example Square, Springfield"]).reasons, ["number_not_listed"]);
});

test("the caller's unit comes back as written, and a country the texts show is read", () => {
  const e = m.pickExplained("350 Example Ave Apt 3, Exampleton, IL 60000", ["350 Example Ave, Exampleton, IL 60000, USA"]);
  assert.equal(e.outcome, "match");
  assert.equal(e.unit, "Apt 3");
  assert.equal(e.inferred_country, "US");
  const given = m.pickExplained("1 Example St #4B, Exampleton, IL", ["1 Example St, Exampleton, IL"], { country: "US" });
  assert.equal(given.unit, "#4B");
  assert.equal(given.inferred_country, null);
});

test("no rows is none, and a row holding the separator is refused", () => {
  assert.deepEqual(m.pickExplained("10 Example St", []), { outcome: "none", index: null, same: [], reasons: [], notes: [], cross_script: false, prefer_on_read: null, rows: [], unit: null, inferred_country: null });
  assert.throws(() => m.pickExplained("10 Example St", ["a\u0001b"]), TypeError);
});

test("many calls keep memory flat", () => {
  const rows = Array.from({ length: 20 }, (_, i) => `${i + 1} Example St, Springfield, IL 62701`);
  m.pickExplained("10 Example St, Springfield, IL 62701", rows, { country: "US" });
  const before = m.memoryBytes();
  for (let i = 0; i < 2000; i++) m.pickExplained("10 Example St, Springfield, IL 62701", rows, { country: "US" });
  assert.equal(m.memoryBytes(), before);
});

test("null options are no options", () => {
  assert.equal(m.pickExplained("10 Example St, Springfield", ["10 Example St, Springfield, IL"], null).outcome, "match");
});

test("input over the caps is refused, not cut", () => {
  const long = "1 " + "Example ".repeat(80);
  assert.ok(long.length > 512);
  assert.throws(() => m.pickExplained(long, ["10 Example St"]), RangeError);
  assert.throws(() => m.pickExplained("10 Example St", [long]), RangeError);
  assert.throws(() => m.pickExplained("10 Example St", [{ text: long, categorized: true }]), RangeError);
  assert.throws(() => m.pickExplained("10 Example St", Array(101).fill("10 Example St")), RangeError);
  assert.throws(() => m.pick(long, ["10 Example St"]), RangeError);
  assert.throws(() => m.compare("10 Example St", long), RangeError);
  // at the caps it answers
  assert.equal(m.pickExplained("x".repeat(512), Array(100).fill("y".repeat(512))).outcome, "none");
});

test("a country hint is read in any case, \"UK\" is the United Kingdom, and an unknown one is no hint", () => {
  for (const country of ["GB", "gb", "UK", "uk"]) {
    assert.equal(m.pickExplained("10 Example St, London", ["10 Example St, London, KY 40741"], { country }).outcome, "none");
    assert.equal(m.pickExplained("10 Example St, London", ["10 Example St, London SW1A 1AA"], { country }).outcome, "match");
  }
  const unknown = m.pickExplained("10 Example St, London, United Kingdom", ["10 Example St, London, KY 40741"], { country: "XX" });
  assert.equal(unknown.outcome, "none");
  assert.equal(unknown.inferred_country, "GB");
});

test("a page of long rows at the caps answers quickly", () => {
  const text = "UK, ".repeat(128);
  const start = performance.now();
  m.pickExplained(text, Array(100).fill(text), { country: "US" });
  assert.ok(performance.now() - start < 3000);
});

test("the picker checks answer from JavaScript", () => {
  assert.equal(m.isControl("Example Park, Springfield", "See more results for Example Park, Springfield"), true);
  assert.equal(m.isControl("Example Park, Springfield", "Jane Doe Example Park, Springfield"), false);
  assert.equal(m.showsTypedText("Example Park, Springfield", "The Example Park, Springfield"), true);
  assert.equal(m.showsTypedText("Example Park, Springfield", "The Example Park, Springfield", 2), false);
  assert.equal(m.inArea("Example Park, Springfield, IL", "Example Gardens, Springfield, IL", "US"), true);
  assert.equal(m.inArea("Example Park, Springfield, IL", "Example Gardens, Springfield, MO", "US"), false);
  assert.equal(m.sameButPostcode("10 Main St, Springfield, IL 62701", "10 Main St, Springfield, IL 62702", "US"), true);
  assert.equal(m.sameButPostcode("10 Main St, Springfield, IL 62701", "10 Main St, Springfield, MO 65801", "US"), false);
  assert.throws(() => m.isControl("x".repeat(513), "y"), RangeError);
});

test("an input that once trapped answers, and memory stays flat", () => {
  for (const [q, rows, cc] of [["5 Sakura, Exampleshi", ["桜ー 5, 例市"], "JP"], ["1 Example, Higashi", ["1 Example, 東ッ"], "JP"], ["Sakura 5", ["桜ー5"], undefined]]) {
    m.pick(q, rows, cc);
    m.pickExplained(q, rows, { country: cc });
  }
  const before = m.memoryBytes();
  for (let i = 0; i < 500; i++) {
    m.pickExplained("5 Sakura, Exampleshi", ["桜ー 5, 例市"], { country: "JP" });
    assert.throws(() => m.pick("10 Example St", ["a\u0001b"]), TypeError);
  }
  assert.equal(m.memoryBytes(), before);
  assert.deepEqual(m.pick("10 Main St, Springfield", ["10 Main St, Springfield, IL"], "US"), { outcome: "match", index: 0 });
});

test("a call that traps replaces the module instance", async () => {
  const { load } = await import("./placematch.mjs");
  // a stand-in module whose am_pick counts its calls in a global and traps on the second
  const name = (s) => [s.length, ...Buffer.from(s)];
  const section = (id, body) => [id, body.length, ...body];
  const pickBody = [0x00, 0x23, 0x00, 0x41, 0x01, 0x6a, 0x24, 0x00, 0x23, 0x00, 0x41, 0x02, 0x46, 0x04, 0x40, 0x00, 0x0b, 0x23, 0x00, 0x0b];
  const bytes = new Uint8Array([
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
    ...section(1, [0x04, 0x60, 0x00, 0x00, 0x60, 0x01, 0x7f, 0x01, 0x7f, 0x60, 0x02, 0x7f, 0x7f, 0x00, 0x60, 0x06, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x01, 0x7f]),
    ...section(3, [0x04, 0x00, 0x01, 0x02, 0x03]),
    ...section(5, [0x01, 0x00, 0x01]),
    ...section(6, [0x01, 0x7f, 0x01, 0x41, 0x00, 0x0b]),
    ...section(7, [0x05, ...name("memory"), 0x02, 0x00, ...name("am_init"), 0x00, 0x00, ...name("am_alloc"), 0x00, 0x01, ...name("am_free"), 0x00, 0x02, ...name("am_pick"), 0x00, 0x03]),
    ...section(10, [0x04, 0x02, 0x00, 0x0b, 0x04, 0x00, 0x41, 0x10, 0x0b, 0x02, 0x00, 0x0b, pickBody.length, ...pickBody]),
  ]);
  const f = await load(bytes);
  assert.deepEqual(f.pick("10 Example St", ["10 Example St"]), { outcome: "match", index: 1 });
  assert.throws(() => f.pick("10 Example St", ["10 Example St"]), WebAssembly.RuntimeError);
  // the same instance would answer 3; a fresh one starts its count again
  assert.deepEqual(f.pick("10 Example St", ["10 Example St"]), { outcome: "match", index: 1 });
});

test("pickExplained takes a town written with its district only under the district rule, never in the US", () => {
  const row = ["Beispielweg 3, 12345 Exampleburg-Nord, Germany"];
  const q = "Beispielweg 3, 12345 Exampleburg, Germany";
  assert.equal(m.pickExplained(q, row, { country: "DE" }).outcome, "none");
  assert.equal(m.pickExplained(q, row, { country: "DE", rules: { districtMayDiffer: false } }).outcome, "none");
  const on = m.pickExplained(q, row, { country: "DE", rules: { districtMayDiffer: true } });
  assert.deepEqual([on.outcome, on.index], ["match", 0]);
  const us = ["1 Example St, Exampleton-North, IL 60601"];
  const usQuery = "1 Example St, Exampleton, IL 60601";
  assert.deepEqual(
    m.pickExplained(usQuery, us, { country: "US", rules: { districtMayDiffer: true } }),
    m.pickExplained(usQuery, us, { country: "US" }),
  );
});
