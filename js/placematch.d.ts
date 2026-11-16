/** A row a site offers: its text, or its text and whether the site shows it with a category of
 * its own (a venue type such as "Convention center"), as it does not for a dropped pin. */
export type Row = string | { text: string; categorized?: boolean };

/** Why an ambiguous answer asks. */
export type AmbiguityReason =
  | "several_places"
  | "place_name_variant"
  | "unit_not_given"
  | "postcode_differs"
  | "number_not_listed"
  | "number_range";

/** Said beside a match: the row leaves out a unit the caller gave. */
export type MatchNote = "unit_not_listed";

export interface RowReport {
  verdict: "same" | "different" | "unknown";
  /** Such as "house_number", "postcode", "place_name", "area", "control", "unit_not_listed",
   * "cross_script", or an ambiguity reason for a near row. */
  reasons: string[];
}

export interface Explained {
  outcome: "match" | "none" | "ambiguous";
  /** match: the row PlaceMatch takes. */
  index: number | null;
  /** match: every row that is the caller's place, in site order; ambiguous: the rows to choose
   * between; none: empty. */
  same: number[];
  /** ambiguous: why. */
  reasons: AmbiguityReason[];
  /** match: notes. */
  notes: MatchNote[];
  /** match: the row was read across scripts. */
  cross_script: boolean;
  /** ambiguous between a same-name row and a longer form of the name: the one of them the site
   * categorized, which a read may take. A write still asks. */
  prefer_on_read: number | null;
  rows: RowReport[];
}

export interface PlaceMatch {
  timings: { compileMs: number; instantiateMs: number; initMs: number };
  memoryBytes(): number;
  pick(query: string, candidates: string[], country?: string): { outcome: "match"; index: number } | { outcome: "none" | "ambiguous"; index: null };
  compare(query: string, candidate: string, country?: string): "same" | "different" | "unknown";
  pickExplained(query: string, rows: Row[], options?: { country?: string }): Explained;
}

/** Load PlaceMatch from WebAssembly bytes. */
export function load(bytes: ArrayBuffer | ArrayBufferView): Promise<PlaceMatch>;
