#!/bin/sh
# Pack the JS package as a git dependency would see it, install the packed copy into a scratch
# project by hand (no registry, no network) and load it by name from Node ESM.
set -eu
cd "$(dirname "$0")/.."
test -f js/placematch.wasm || { echo "js/placematch.wasm is missing: run tools/build-js.sh" >&2; exit 1; }
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
npm pack --silent --pack-destination "$scratch" > "$scratch/name"
mkdir -p "$scratch/app/node_modules/@pomerado/placematch"
tar -xzf "$scratch/$(cat "$scratch/name")" -C "$scratch/app/node_modules/@pomerado/placematch" --strip-components=1
echo "packed files:"; (cd "$scratch/app/node_modules/@pomerado/placematch" && find . -type f | sort)
cat > "$scratch/app/check.mjs" <<'JS'
import { open } from "@pomerado/placematch";
import { load } from "@pomerado/placematch/loader";
import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
const m = await open();
const e = m.pickExplained("10 Example St, Apt 4, Springfield, IL", ["10 Example St, Springfield, IL 62701"], { country: "US" });
if (e.outcome !== "match" || e.notes[0] !== "unit_not_listed") throw new Error("unexpected answer: " + JSON.stringify(e));
const wasm = createRequire(import.meta.url).resolve("@pomerado/placematch/placematch.wasm");
const m2 = await load(await readFile(wasm));
if (m2.pick("10 Example St, Springfield", ["10 Example St, Springfield, IL"], "US").outcome !== "match") throw new Error("loader entry failed");
console.log("loaded by name:", JSON.stringify(e));
JS
(cd "$scratch/app" && echo '{"type":"module"}' > package.json && node check.mjs)
