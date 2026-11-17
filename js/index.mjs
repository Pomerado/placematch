// Node entry: loads the WebAssembly module shipped beside this file. No network access.
import { readFile } from "node:fs/promises";
import { load, LIMITS } from "./placematch.mjs";

export { load, LIMITS };

/** Load PlaceMatch from the package's own placematch.wasm. Load once and reuse the result. */
export async function open() {
  let bytes;
  try {
    bytes = await readFile(new URL("./placematch.wasm", import.meta.url));
  } catch (error) {
    if (error?.code === "ENOENT") {
      throw new Error("placematch.wasm is not in this copy of PlaceMatch: depend on a release tag, or run tools/build-js.sh", { cause: error });
    }
    throw error;
  }
  return load(bytes);
}
