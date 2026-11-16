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
  | "number_range"
  | "place_unconfirmed";

/** Said beside a match: the row leaves out a unit the caller gave. */
export type MatchNote = "unit_not_listed";

/** The longest query or row text (JavaScript string length) and the most rows a call takes; longer
 * input throws a RangeError and is never cut. */
export const LIMITS: { readonly textLength: 512; readonly rows: 100 };

export interface RowReport {
  verdict: "same" | "different" | "unknown";
  /** Such as "house_number", "postcode", "place_name", "area", "control", "unit_not_listed",
   * "cross_script", or an ambiguity reason for a near row. */
  reasons: string[];
  /** For each row of the caller's place: "unit_not_listed" when it leaves out the caller's unit. */
  notes: MatchNote[];
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
  /** match: notes for the row at `index`; each row has its own in `rows[i].notes`. */
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
  pickExplained(query: string, rows: Row[], options?: { country?: string } | null): Explained;
  /** The row is a site's control ("See more results for <text>", "Search "<text>""), never a place. */
  isControl(query: string, row: string): boolean;
  /** The row shows the caller's whole text (of two parts or more) after at least `minWords` (default
   * 1) words of its own first part ("The <text>"). */
  showsTypedText(query: string, row: string, minWords?: number): boolean;
  /** The row, after its first part, is in the area the caller gave after theirs. */
  inArea(query: string, row: string, country?: string): boolean;
  /** The caller's house, street and town with only the postcode different, in the same postal area
   * and district: the same building under another postcode, which a read may accept. */
  sameButPostcode(query: string, row: string, country?: string): boolean;
}

/** Load PlaceMatch from WebAssembly bytes. */
export function load(bytes: ArrayBuffer | ArrayBufferView): Promise<PlaceMatch>;
