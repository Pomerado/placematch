export * from "./placematch.js";
import type { PlaceMatch } from "./placematch.js";

/** Load PlaceMatch from the package's own placematch.wasm. */
export function open(): Promise<PlaceMatch>;
