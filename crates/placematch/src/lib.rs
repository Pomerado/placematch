//! placematch: decide whether a site's address suggestion is the same place as the caller's address.
//!
//! Pure Rust, no network, no global state. Data (libpostal dictionaries, libaddressinput country
//! data, OpenCage region names) is embedded at build time and parsed into a `Matcher` once.
//!
//! ```
//! let m = placematch::Matcher::new();
//! let r = m.pick("10 Main Street, Springfield", &["10 Main St, Springfield, IL", "100 Main St, Springfield"], Some("US"));
//! assert_eq!(r, placematch::Pick::Match(0));
//! ```
pub mod data;
pub mod fold;
pub mod parse;
pub mod reading;
pub mod token;

use fold::Script;
pub use parse::Components;
use parse::{Ctx, Text};


/// Dictionaries and country data, pre-folded by build.rs.
pub const EMBEDDED: &str = include_str!(concat!(env!("OUT_DIR"), "/placematch-data.txt"));

pub struct Matcher {
    data: data::Data,
    cjk: token::CjkRules,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Same,
    Different,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    Match(usize),
    None,
    Ambiguous(Vec<usize>),
}

/// A row a site offers for the caller's text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row<'a> {
    /// The row's text as the site shows it.
    pub text: &'a str,
    /// The site shows the row with a category of its own (a venue type such as "Convention
    /// center"), as it does for a listed venue and not for a dropped pin or a plain address.
    pub categorized: bool,
}

impl<'a> From<&'a str> for Row<'a> {
    fn from(text: &'a str) -> Self {
        Row { text, categorized: false }
    }
}

impl<'a> From<&'a String> for Row<'a> {
    fn from(text: &'a String) -> Self {
        Row { text, categorized: false }
    }
}

/// Optional matching rules for `Matcher::pick_explained_with`, each off by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rules {
    /// Outside the US and its territories, a row that writes the caller's town with a district after it
    /// ("Exampleburg-Nord" for "Exampleburg") is the caller's town when both give one postcode.
    /// Off, such a row is another town.
    pub district_may_differ: bool,
}

thread_local! {
    /// `Rules::district_may_differ` for the pick under way on this thread.
    static DISTRICT_MAY_DIFFER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Options for `Matcher::pick_explained`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PickOptions<'a> {
    /// The site's country (ISO 3166-1 alpha-2, in any case; "UK" is "GB"), when known. A code the
    /// data does not know is read as none, so the country is read from the texts.
    pub country: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// One place among the rows is the caller's.
    Match,
    /// No row is the caller's place.
    None,
    /// The caller should choose: see `Explained::reasons`.
    Ambiguous,
}

/// Why an `ambiguous` answer asks, in this order:
/// - `several_places`: two or more different places fit (the same street and number in two towns,
///   or a row that may be the caller's street in another place);
/// - `place_name_variant`: the caller named a place, and a row holds the name with other words added
///   (a venue under its longer official name), so it may be the place;
/// - `place_unconfirmed`: the caller named a place, and a row with the name does not state the
///   caller's town or street, or (with no country given) adds a place the caller did not name
///   ("CA" may be California or Canada); a row giving the caller's town in another script whose
///   reading several towns share; or, only when no row is the caller's place, a row at the caller's
///   number and street that leaves out the caller's town, or the state the caller gave; or a row
///   (or a caller's text) too long to read whole that lost a number or unit word (see `MAX_PARTS`);
/// - and, only when no row is the caller's place, the near rows: they add a unit the caller did not
///   give (`unit_not_given`), show another postcode at the caller's number, street and town
///   (`postcode_differs`), list the caller's street without a house number (`number_not_listed`), or
///   give a house-number range holding the caller's number with no postcode on both sides to confirm
///   it (`number_range`; ranges are read only where `RANGE_COUNTRIES` allows).
pub const AMBIGUITY_REASONS: &[&str] = &["several_places", "place_name_variant", "unit_not_given", "postcode_differs", "number_not_listed", "number_range", "place_unconfirmed"];

/// Notes beside a `match`: `unit_not_listed` when the row leaves out a unit the caller gave (a
/// picker types the unit into the site's own unit field).
pub const MATCH_NOTES: &[&str] = &["unit_not_listed"];

/// One row's verdict against the caller's text, with its reasons ("house_number", "postcode",
/// "place_name", "area", "control", "house_number_range", "cross_script", or a near row's ambiguity
/// reason) and, for each row of the caller's place, its notes from `MATCH_NOTES`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowReport {
    pub verdict: Verdict,
    pub reasons: Vec<&'static str>,
    /// `unit_not_listed` when this row leaves out a unit the caller gave, so a picker taking any
    /// row of the place knows whether to type the unit.
    pub notes: Vec<&'static str>,
}

/// `Matcher::pick_explained`'s answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Explained {
    pub outcome: Outcome,
    /// `match`: the row PlaceMatch takes (the row `pick` returns).
    pub index: Option<usize>,
    /// `match`: every row that is the caller's place, in site order, so a picker may take the site's
    /// own first; `ambiguous`: the rows to choose between; `none`: empty.
    pub same: Vec<usize>,
    /// `ambiguous`: why, from `AMBIGUITY_REASONS`.
    pub reasons: Vec<&'static str>,
    /// `match`: notes from `MATCH_NOTES` for the row at `index`; each row of `same` has its own in
    /// `rows[i].notes`.
    pub notes: Vec<&'static str>,
    /// `match`: the row was read across scripts ("Ulitsa Primer" for "Улица Пример", "Asahicho" for
    /// "旭町").
    pub cross_script: bool,
    /// `ambiguous` between a same-name row and a row with the name in a longer form: the one row of
    /// them the site categorized, when exactly one is, the caller gave a town or postcode and every
    /// such row holds it in full (or the exact name is only outside it). A read may take it; a write
    /// still asks; with no area from the caller there is none.
    pub prefer_on_read: Option<usize>,
    /// Each row's verdict and reasons, by row index.
    pub rows: Vec<RowReport>,
    /// The caller's unit as the caller wrote it ("Apt 3", "#4B", "Suite 2"; several are joined with
    /// ", "), whatever the outcome, so a picker can type it into the site's own unit field; `None`
    /// when the caller gave none.
    pub unit: Option<String>,
    /// With no country given: the country the texts were read by, when they show one (see
    /// `Matcher::infer_country`); `None` when the caller gave one or the texts do not show one.
    pub inferred_country: Option<String>,
}

impl Explained {
    fn none(rows: Vec<RowReport>) -> Explained {
        Explained { outcome: Outcome::None, index: None, same: vec![], reasons: vec![], notes: vec![], cross_script: false, prefer_on_read: None, rows, unit: None, inferred_country: None }
    }

    fn ambiguous(same: Vec<usize>, reasons: Vec<&'static str>, rows: Vec<RowReport>) -> Explained {
        let mut reasons = reasons;
        reasons.sort_by_key(|r| AMBIGUITY_REASONS.iter().position(|x| x == r));
        reasons.dedup();
        Explained { outcome: Outcome::Ambiguous, index: None, same, reasons, notes: vec![], cross_script: false, prefer_on_read: None, rows, unit: None, inferred_country: None }
    }

    /// The answer as one line of JSON: `outcome` ("match", "none", "ambiguous"), `index`, `same`,
    /// `reasons`, `notes`, `cross_script`, `prefer_on_read`, `rows` (each `verdict` and `reasons`),
    /// `unit` and `inferred_country` (a string or null).
    pub fn to_json(&self) -> String {
        fn words(v: &[&str]) -> String {
            format!("[{}]", v.iter().map(|w| format!("\"{}\"", w)).collect::<Vec<_>>().join(","))
        }
        fn opt(v: Option<usize>) -> String {
            v.map_or("null".to_string(), |i| i.to_string())
        }
        let outcome = match self.outcome {
            Outcome::Match => "match",
            Outcome::None => "none",
            Outcome::Ambiguous => "ambiguous",
        };
        let rows: Vec<String> = self
            .rows
            .iter()
            .map(|r| {
                let v = match r.verdict {
                    Verdict::Same => "same",
                    Verdict::Different => "different",
                    Verdict::Unknown => "unknown",
                };
                format!("{{\"verdict\":\"{}\",\"reasons\":{},\"notes\":{}}}", v, words(&r.reasons), words(&r.notes))
            })
            .collect();
        fn text(v: &Option<String>) -> String {
            let Some(t) = v else {
                return "null".to_string();
            };
            let mut o = String::from("\"");
            for ch in t.chars() {
                match ch {
                    '"' => o.push_str("\\\""),
                    '\\' => o.push_str("\\\\"),
                    c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
                    c => o.push(c),
                }
            }
            o.push('"');
            o
        }
        format!(
            "{{\"outcome\":\"{}\",\"index\":{},\"same\":[{}],\"reasons\":{},\"notes\":{},\"cross_script\":{},\"prefer_on_read\":{},\"rows\":[{}],\"unit\":{},\"inferred_country\":{}}}",
            outcome,
            opt(self.index),
            self.same.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","),
            words(&self.reasons),
            words(&self.notes),
            self.cross_script,
            opt(self.prefer_on_read),
            rows.join(","),
            text(&self.unit),
            text(&self.inferred_country)
        )
    }
}

/// Scripts that name the same thing only through a reading: Latin (with digits and punctuation),
/// Han and Kana together, and each other script on its own.
fn script_group(s: Option<Script>) -> u8 {
    match s {
        None | Some(Script::Latin) | Some(Script::Digit) | Some(Script::Other) => 0,
        Some(Script::Han) | Some(Script::Kana) => 1,
        Some(x) => 2 + x as u8,
    }
}

/// Countries where a listing's hyphenated pair of house numbers ("10-14 Example St") is a range of
/// one building's numbers. Elsewhere it may be a unit and its number (Australia, New Zealand,
/// Canada), a block and lot, or a number and its addition, so it is never read as a range.
const RANGE_COUNTRIES: &[&str] = &["US", "GB", "IE", "DE", "FR"];

/// The widest range read as one building. A building's numbers on one side of a street span a few
/// consecutive same-side numbers ("10-14", "2-8"); 8 allows five of them. Street-segment ranges a
/// geocoder lists ("100-198") and zone or block forms ("91-1234") are far wider.
const RANGE_WIDTH: u32 = 8;

/// A listing's house-number range ("10-14") holds the caller's number: ascending, at most
/// `RANGE_WIDTH` wide, both ends and the number on one side of the street (all even or all odd).
fn range_holds(range: &str, number: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.len() <= 6 && s.bytes().all(|b| b.is_ascii_digit());
    let Some((lo, hi)) = range.split_once('-') else {
        return false;
    };
    if !digits(lo) || !digits(hi) || !digits(number) {
        return false;
    }
    let (lo, hi, n): (u32, u32, u32) = (lo.parse().unwrap(), hi.parse().unwrap(), number.parse().unwrap());
    lo < hi && hi - lo <= RANGE_WIDTH && lo % 2 == hi % 2 && n % 2 == lo % 2 && lo <= n && n <= hi
}

/// Words of a site's control row ("See more results for ...", "Search ...").
const CONTROL_WORDS: &[&str] = &["see", "search", "searching", "show", "more", "results", "result", "use", "find", "browse", "everything", "matches", "suggestions"];
/// Words a control row may hold besides control words and counts.
const CONTROL_FILLERS: &[&str] = &["for", "all", "in", "of", "the", "my", "a", "an", "to", "with", "about", "other", "on", "near"];
/// Countries whose two-letter state or province codes follow a town in an address ("Paris, TX").
/// The state or territory of each range of United States ZIP code prefixes (the first three digits,
/// as the US Postal Service assigns them), by code. One prefix names one state, except a few shared
/// with a territory or a town across a state line (063, 008, 967, 969). A prefix not listed
/// confirms and contradicts nothing.
const US_ZIP_PREFIXES: &[(u16, u16, &str)] = &[
    (5, 5, "ny"), (6, 7, "pr"), (8, 8, "pr vi"), (9, 9, "pr"), (10, 27, "ma"), (28, 29, "ri"), (30, 38, "nh"),
    (39, 49, "me"), (50, 54, "vt"), (55, 55, "ma"), (56, 59, "vt"), (60, 62, "ct"), (63, 63, "ct ny"),
    (64, 69, "ct"), (70, 89, "nj"), (90, 98, "ae"), (100, 149, "ny"), (150, 196, "pa"),
    (197, 199, "de"), (200, 200, "dc"), (201, 201, "va"), (202, 205, "dc"), (206, 219, "md"),
    (220, 246, "va"), (247, 268, "wv"), (270, 289, "nc"), (290, 299, "sc"), (300, 319, "ga"),
    (320, 339, "fl"), (340, 340, "aa"), (341, 349, "fl"), (350, 369, "al"), (370, 385, "tn"),
    (386, 397, "ms"), (398, 399, "ga"), (400, 427, "ky"), (430, 459, "oh"), (460, 479, "in"),
    (480, 499, "mi"), (500, 528, "ia"), (530, 549, "wi"), (550, 567, "mn"), (569, 569, "dc"),
    (570, 577, "sd"), (580, 588, "nd"), (590, 599, "mt"), (600, 629, "il"), (630, 658, "mo"),
    (660, 679, "ks"), (680, 693, "ne"), (700, 714, "la"), (716, 729, "ar"), (730, 732, "ok"),
    (733, 733, "tx"), (734, 749, "ok"), (750, 799, "tx"), (800, 816, "co"), (820, 831, "wy"),
    (832, 838, "id"), (840, 847, "ut"), (850, 865, "az"), (870, 884, "nm"), (885, 885, "tx"),
    (889, 898, "nv"), (900, 961, "ca"), (962, 966, "ap"), (967, 967, "hi as"), (968, 968, "hi"),
    (969, 969, "gu mp as pw fm mh"), (970, 979, "or"), (980, 994, "wa"), (995, 999, "ak"),
];

/// Street types that end the names of towns more often than of streets ("Example Grove", "Spring
/// Valley", "Palm Beach Gardens"), each by the first word of its dictionary line. In a postcode's
/// first part with no comma after the code, every other street type makes that part a street ("92999
/// Example Loop Spring Valley"), so a type missing here reads as a street, never as a town. A name
/// then three or more of these, or four or more of them alone, is a street then its town ("92999
/// Example Grove Spring Valley", "92999 Spring Grove Spring Valley"); a name then one or two of them
/// reads as a town ("92999 Example Grove Spring").
const TOWN_TYPES: &[&str] = &[
    "grove", "garden", "gardens", "park", "valley", "bay", "spring", "springs", "beach", "hill", "hills", "heights", "village", "falls", "island", "islands", "harbor",
];

/// `TOWN_TYPES` by the dictionary's canonical keys.
fn town_type_keys() -> &'static [String] {
    static KEYS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    KEYS.get_or_init(|| TOWN_TYPES.iter().map(|t| data::concat_key(t)).collect())
}

const POSTAL_CODE_COUNTRIES: &[&str] = &["US", "CA", "AU"];

/// The most comma parts, and characters, of a text that are read: an address has far fewer, and a
/// site's row, which the caller does not control, may have many, while matching time grows with
/// them. A longer text first loses middle parts that hold no number and no unit word (a facility's
/// wing or department), keeping its first part and last `KEPT_LAST_PARTS` (the town, region,
/// postcode and country); one still too long keeps its first parts (`KEPT_FIRST_PARTS`) or
/// characters (`KEPT_FIRST_CHARS`) and its last ones (see `Matcher::bound`).
const MAX_PARTS: usize = 16;
const KEPT_FIRST_PARTS: usize = 4;
const KEPT_LAST_PARTS: usize = 5;
const MAX_CHARS: usize = 256;
const KEPT_FIRST_CHARS: usize = 96;

/// The most distinct rows of one name or address that are compared with each other to tell whether
/// they are one place (pairs grow with the square of the rows); more are asked about.
const MAX_COMPARED_ROWS: usize = 16;

/// A text as it is read: folded, with its spaces collapsed ("10  Main St" and "10 main st" are one).
fn text_key(t: &str) -> String {
    fold::fold(t).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The number of distinct texts (see `text_key`).
fn distinct_texts<'a>(texts: impl Iterator<Item = &'a str>) -> usize {
    let mut seen: Vec<String> = Vec::new();
    for t in texts {
        let k = text_key(t);
        if !seen.contains(&k) {
            seen.push(k);
        }
    }
    seen.len()
}

/// Countries whose postcodes are written in their one form, so a postcode of another form is
/// another country's.
const STRICT_POSTCODE_COUNTRIES: &[&str] = &["US", "CA", "GB", "AU"];
/// Cities that repeat street names across districts and postcodes.
const REPEATED_STREET_CITIES: &[&str] = &["berlin", "hamburg", "london", "wien", "vienna", "budapest", "praha", "prague", "brussels", "bruxelles", "brussel", "antwerpen", "antwerp", "anvers", "istanbul"];

/// How a row's area relates to the caller's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Area {
    Fail,
    Ask,
    /// `full`: the row holds every area part the caller gave
    Fits { full: bool },
}

/// Words that name a transport place, whose name a site may follow with its three-letter code
/// ("Example International Airport (EXA)"), in the languages most such codes are written beside.
const TRANSPORT_WORDS: &[&str] = &["airport", "aerodrome", "airfield", "heliport", "station", "terminal", "aeroport", "aeropuerto", "aeroporto", "flughafen", "luchthaven", "bahnhof", "hauptbahnhof", "gare", "estacion", "estacao", "stazione"];

/// The three-letter compass points, which a short code is never ("Example Station (NNE)").
const COMPASS_POINTS: &[&str] = &["nne", "ene", "ese", "sse", "ssw", "wsw", "wnw", "nnw"];

/// Age and size words that, before a place's name, may name another place ("Old Example Park",
/// "Upper Example Park"), in the languages PlaceMatch reads; compass words come from the dictionaries.
const AGE_WORDS: &[&str] = &[
    "old", "new", "upper", "lower", "little", "great", "vieux", "vieil", "vieille", "nouveau", "nouvel", "nouvelle", "haut", "haute", "bas", "basse",
    "petit", "petite", "grand", "grande", "viejo", "vieja", "nuevo", "nueva", "alto", "alta", "bajo", "baja", "alt", "alte", "alten", "neu", "neue",
    "neuen", "ober", "obere", "unter", "untere", "klein", "kleine", "gross", "grosse", "vecchio", "vecchia", "nuovo", "nuova", "piccolo", "piccola",
    "velho", "velha", "novo", "nova", "oud", "oude", "nieuw", "nieuwe", "stary", "stara", "nowy", "nowa",
];


#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tri {
    Yes,
    No,
    Unknown,
}

impl Default for Matcher {
    fn default() -> Self {
        Self::new()
    }
}

/// At most one edit, never in a digit ("2e" and "12e" are different districts).
fn dl_le1(a: &str, b: &str) -> bool {
    let da: String = a.chars().filter(|c| c.is_ascii_digit()).collect();
    let db: String = b.chars().filter(|c| c.is_ascii_digit()).collect();
    da == db && dl_le1_raw(a, b)
}

/// Damerau-Levenshtein distance at most 1.
fn dl_le1_raw(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if (a.len() as isize - b.len() as isize).abs() > 1 {
        return false;
    }
    if a == b {
        return true;
    }
    let mut i = 0;
    while i < a.len() && i < b.len() && a[i] == b[i] {
        i += 1;
    }
    let (ra, rb) = (&a[i..], &b[i..]);
    if ra.len() == rb.len() {
        // substitution or transposition
        if ra[1..] == rb[1..] {
            return true;
        }
        if ra.len() >= 2 && ra[0] == rb[1] && ra[1] == rb[0] && ra[2..] == rb[2..] {
            return true;
        }
        false
    } else if ra.len() > rb.len() {
        ra[1..] == *rb
    } else {
        rb[1..] == *ra
    }
}

/// One edit (insert, delete, substitute, swap) between the caller's name `toks` and `b`; the policy
/// allows it for names of 6+ letters.
/// Street type words that translate one another across scripts, as OpenStreetMap's English names
/// and local signs write them ("Zamkova Street" / "Замкова вулиця", "Al Zaghl Street" / "شارع الزغل").
const TYPE_GROUPS: &[&[&str]] = &[
    &["street", "st", "str", "شارع", "ش", "רחוב", "רח", "улица", "ул", "вулиця", "вул", "οδός", "οδ", "ถนน", "ถ", "улица", "ulitsa", "vulytsia", "odos", "shari", "thanon", "街", "jie", "gai"],
    &["road", "rd", "طريق", "דרך", "шоссе", "шосе", "ถนน", "tariq", "derekh", "thanon", "路", "lu", "通", "tori", "dori", "로", "ro", "lo"],
    &["avenue", "ave", "av", "проспект", "просп", "пр", "λεωφόρος", "λεωφ", "שדרות", "שד", "prospekt", "prospekt", "leoforos", "sderot", "大道", "dadao", "대로", "daero", "daelo"],
    &["boulevard", "blvd", "бульвар", "бул", "бульв", "bulvar", "bulevard", "大道", "dadao"],
    &["lane", "ln", "alley", "soi", "ซอย", "переулок", "пер", "провулок", "пров", "סמטה", "زقاق", "pereulok", "provulok", "巷", "xiang", "hang", "弄", "nong", "long", "길", "gil"],
    &["square", "sq", "plaza", "площадь", "пл", "площа", "כיכר", "ميدان", "πλατεία", "ploshchad", "ploshcha", "kikar", "midan", "plateia"],
];

fn same_type_group(a: &str, b: &str) -> bool {
    use std::sync::OnceLock;
    static KEYS: OnceLock<Vec<Vec<String>>> = OnceLock::new();
    let keys = KEYS.get_or_init(|| TYPE_GROUPS.iter().map(|g| g.iter().map(|w| fold::ascii_key(&fold::fold(w))).filter(|k| !k.is_empty()).collect()).collect());
    keys.iter().any(|g| g.iter().any(|k| k == a) && g.iter().any(|k| k == b))
}

/// `a` appears inside `b`, and is long enough to mean something ("shi" alone does not).
fn within(a: &str, b: &str) -> bool {
    a.len() >= 4 && b.contains(a)
}

/// Nordic street types that take a definite ending ("vei" / "veien", "gata" / "gatan").
const NORDIC_TYPES: &[&str] = &["vei", "veg", "vag", "vej", "gata", "gate", "gade", "gat", "plass", "plads", "plats", "torg", "torv", "sti", "stig", "bakke", "backe", "alle", "kuja", "tie", "katu"];

fn typo_in_long_word(toks: &[&str], b: &str) -> bool {
    let a: String = toks.concat();
    a != b && letters(&a).max(letters(b)) >= 6 && dl_le1(&a, b)
}

fn letters(s: &str) -> usize {
    s.chars().filter(|c| c.is_ascii_alphabetic()).count()
}

fn intersects(a: &[u32], b: &[u32]) -> bool {
    a.iter().any(|x| b.contains(x))
}

fn is_latinish(s: Option<Script>) -> bool {
    matches!(s, None | Some(Script::Latin) | Some(Script::Digit) | Some(Script::Other))
}

/// Compass words of a name, as one sorted list of canonical keys ("N", "North", "Nord" alike).
fn directions(t: &Text) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = t
        .toks
        .iter()
        .filter_map(|(k, _)| {
            Some(match k.as_str() {
                "n" | "north" | "nord" | "norte" | "noord" => "n",
                "s" | "south" | "sud" | "sur" | "zuid" | "sued" => "s",
                "e" | "east" | "est" | "este" | "oost" | "ost" => "e",
                "w" | "west" | "ouest" | "oeste" => "w",
                "ne" | "northeast" => "ne",
                "nw" | "northwest" => "nw",
                "se" | "southeast" => "se",
                "sw" | "southwest" => "sw",
                _ => return None,
            })
        })
        .collect();
    v.sort();
    v
}

/// A name written in Han characters only, in one form ("臺中市" gives "台中市").
fn han_name(t: &Text) -> Option<String> {
    fold::all_han(&t.native).then(|| fold::han_canon(&t.native))
}

/// Two Han names in one form are the same name: the same characters, or one adds a kind suffix the
/// other leaves out ("台中" and "台中市"). Two different suffixes are two places ("鹿児島市", the
/// city, and "鹿児島県", the prefecture).
fn han_eq(a: &str, b: &str) -> bool {
    let plus_suffix = |long: &str, short: &str| long.chars().count() == short.chars().count() + 1 && long.starts_with(short) && long.chars().last().map_or(false, |ch| HAN_ADMIN.contains(&ch)) && !short.is_empty();
    a == b || plus_suffix(a, b) || plus_suffix(b, a)
}

/// The compass words of a street: before its name ("NE Example St"), or right after its type
/// ("Example St NE"), with the rest of the street's words. A compass word after the town in an
/// address written without commas is the town's state or part of the town ("Example St Exampleton
/// NE", "Example St North Exampleton").
fn street_directions(t: &Text) -> (Vec<&'static str>, String) {
    let one = |i: usize| -> Vec<&'static str> {
        let w = Text { toks: vec![t.toks[i].clone()], ..Default::default() };
        directions(&w)
    };
    let mut v = Vec::new();
    if t.toks.is_empty() {
        return (v, String::new());
    }
    let (mut from, mut to) = (0, t.toks.len());
    let first = one(0);
    if !first.is_empty() {
        v.extend(first);
        from = 1;
    }
    let last = t.toks.len() - 1;
    if last > 0 && last >= from {
        let d = one(last);
        let after_type = !t.toks[last - 1].1.is_empty() && one(last - 1).is_empty();
        if !d.is_empty() && after_type {
            v.extend(d);
            // the street type the compass word follows, which the other side may keep apart
            to = last - 1;
        }
    }
    v.sort();
    // the other words: a compass word only on one side of the same words is another street; "S." for
    // "San" is a word the other side spells out ("Av S. Ejemplo", "Avenida San Ejemplo")
    let rest: String = t.toks[from..to.max(from)].iter().map(|x| x.0.as_str()).collect();
    (v, rest)
}

/// Compare two names (street or locality). `typo`: allow one edit for names of 6+ letters.
fn text_eq(m: &Matcher, q: &Text, c: &Text, typo: bool) -> Tri {
    // two Han names are one name only when their characters agree, up to traditional, Japanese and
    // simplified forms: a shared reading is not enough ("福州" and "抚州" both read "fuzhou")
    if let (Some(a), Some(b)) = (han_name(q), han_name(c)) {
        return if han_eq(&a, &b) { Tri::Yes } else { Tri::No };
    }
    if q.key == c.key {
        return Tri::Yes;
    }
    // a different compass word is a different street, never a typo ("S Example" / "N Example")
    let typo = typo && directions(q) == directions(c);
    // a numbered district of the same town ("Paris" vs "Paris 20e Arrondissement")
    for (a, b) in [(&q.key, &c.key), (&c.key, &q.key)] {
        if a.len() >= 3 && b.len() > a.len() && b.starts_with(a.as_str()) && b[a.len()..].starts_with(|ch: char| ch.is_ascii_digit()) {
            return Tri::Yes;
        }
    }
    // token-wise with canonical synonyms and dotted abbreviations
    let ab = |t: &Text, i: usize| t.abbr.get(i).copied().unwrap_or(false);
    if q.toks.len() == c.toks.len() && !q.toks.is_empty() {
        let all = q.toks.iter().zip(&c.toks).enumerate().all(|(i, (a, b))| {
            a.0 == b.0
                || intersects(&a.1, &b.1)
                || (typo && !ab(q, i) && letters(&a.0).max(letters(&b.0)) >= 6 && dl_le1(&a.0, &b.0))
                || (ab(q, i) && parse::is_abbrev(&a.0, &b.0))
                || (ab(c, i) && parse::is_abbrev(&b.0, &a.0))
        });
        if all {
            return Tri::Yes;
        }
    }
    // compound vs abbreviated last word: "knutsengata" vs "knutsen gt."
    for (x, y) in [(q, c), (c, q)] {
        if x.toks.len() >= 2 && ab(x, x.toks.len() - 1) {
            let prefix: String = x.toks[..x.toks.len() - 1].iter().map(|t| t.0.as_str()).collect();
            let last = &x.toks.last().unwrap().0;
            if y.key.len() > prefix.len() && y.key.starts_with(&prefix) && parse::is_abbrev(last, &y.key[prefix.len()..]) {
                return Tri::Yes;
            }
        }
    }
    // canonical concatenation (first canonical of each token, else its key)
    let canon = |t: &Text| -> Vec<String> {
        let mut variants = vec![String::new()];
        for (k, cs) in &t.toks {
            if cs.is_empty() {
                for v in variants.iter_mut() {
                    v.push_str(k);
                }
            } else {
                let mut nv = Vec::new();
                for v in &variants {
                    for cid in cs.iter().take(3) {
                        nv.push(format!("{}{}", v, m.data.canon_keys[*cid as usize]));
                    }
                    nv.push(format!("{}{}", v, k));
                }
                nv.truncate(16);
                variants = nv;
            }
        }
        variants
    };
    let qa = canon(q);
    let ca = canon(c);
    if qa.iter().any(|x| ca.contains(x)) {
        return Tri::Yes;
    }
    let qtoks: Vec<&str> = q.toks.iter().map(|t| t.0.as_str()).collect();
    // a dotted word is an abbreviation, not a typo ("Müllerg." is not "Müller" + one letter)
    if typo && !q.abbr.last().copied().unwrap_or(false) && typo_in_long_word(&qtoks, &c.key) {
        return Tri::Yes;
    }
    let qs = q.script;
    let cs = c.script;
    let cross = is_latinish(qs) != is_latinish(cs) || (qs != cs && !is_latinish(qs) && !is_latinish(cs));
    if cross {
        if !q.skel.is_empty() && q.skel == c.skel {
            return Tri::Yes;
        }
        if qa.iter().any(|x| ca.iter().any(|y| fold::skeleton(x) == fold::skeleton(y) && !fold::skeleton(x).is_empty())) {
            return Tri::Yes;
        }
        // Hebrew and Arabic letters against a romanization ("קרל פופר" / "Karl Popper")
        let abjad = |s: Option<Script>| matches!(s, Some(Script::Hebrew) | Some(Script::Arabic));
        if abjad(qs) || abjad(cs) {
            let heb = matches!(qs, Some(Script::Hebrew)) || matches!(cs, Some(Script::Hebrew));
            let (a, b) = (fold::abjad_key(&q.key, heb), fold::abjad_key(&c.key, heb));
            if a.len() >= 3 && a == b {
                return Tri::Yes;
            }
        }
        // Han and Kana names read with their per-character readings ("台中市" / "Taichung")
        if reading_eq(q, c) {
            return Tri::Yes;
        }
        // other Han/Kana readings cannot be recovered from a romanization
        let hard = |s: Option<Script>| matches!(s, Some(Script::Han) | Some(Script::Kana) | Some(Script::Thai) | Some(Script::Arabic) | Some(Script::Hebrew) | Some(Script::Devanagari));
        if hard(qs) || hard(cs) {
            return Tri::Unknown;
        }
    }
    Tri::No
}

/// Han administrative suffixes a romanization may translate or leave out ("台中市" / "Taichung").
const HAN_ADMIN: &[char] = &['市', '区', '區', '県', '縣', '县', '府', '都', '省', '郡', '鎮', '镇', '鄉', '乡', '村', '町', '道'];
/// Their romanized and English forms.
const LATIN_ADMIN: &[&str] = &["shi", "ku", "ken", "fu", "to", "do", "gun", "son", "mura", "machi", "cho", "city", "district", "ward", "county", "prefecture", "province", "qu", "xian", "sheng", "zhen", "xiang", "town", "township", "village", "hsien", "hsiang", "chen", "shih", "chu", "cun"];

/// One side is Han or Kana and the other a romanization that reads as it.
fn reading_eq(q: &Text, c: &Text) -> bool {
    let ok = |t: &Text| reading::readable(&t.native) || reading::thai(&t.native);
    let (nat, lat) = if ok(q) && is_latinish(c.script) {
        (q, c)
    } else if ok(c) && is_latinish(q.script) {
        (c, q)
    } else {
        return false;
    };
    if reading::thai(&nat.native) {
        let full: String = if lat.native.is_empty() { lat.key.clone() } else { lat.native.chars().filter(|ch| ch.is_ascii_alphabetic()).collect() };
        return full.len() >= 4 && reading::thai_reads_as(&nat.native, &full);
    }
    // the romanization as written: the ASCII key folds "ae" as an umlaut ("Kitaen" is not "Kitan")
    let letters = |t: &Text| -> String { if t.native.is_empty() { t.key.clone() } else { t.native.chars().filter(|ch| ch.is_ascii_alphabetic()).collect() } };
    let full = letters(lat);
    if full.is_empty() || !full.bytes().all(|b| b.is_ascii_lowercase()) {
        return false;
    }
    let mut natives = vec![nat.native.clone()];
    if nat.native.chars().count() >= 2 && nat.native.ends_with(HAN_ADMIN) {
        let mut s = nat.native.clone();
        s.pop();
        natives.push(s);
    }
    let mut lats = vec![full.clone()];
    // a direction translated in the romanization ("南京西" / "Nanjing West")
    let mut natives = natives;
    for (w, h) in [("north", '北'), ("south", '南'), ("east", '東'), ("east", '东'), ("west", '西')] {
        if full.len() > w.len() + 1 && full.ends_with(w) && nat.native.ends_with(h) && nat.native.chars().count() >= 2 {
            let mut n = nat.native.clone();
            n.pop();
            natives.push(n);
            lats.push(full[..full.len() - w.len()].to_string());
        }
    }
    if let Some(last) = lat.toks.last() {
        if lat.toks.len() >= 2 && LATIN_ADMIN.contains(&last.0.as_str()) && full.ends_with(last.0.as_str()) {
            lats.push(full[..full.len() - last.0.len()].to_string());
        }
    }
    for n in &natives {
        for l in &lats {
            if l.len() >= 2 && [reading::Lang::Chinese, reading::Lang::Japanese].iter().any(|&lang| reading::reads_as(n, l, lang)) {
                return true;
            }
        }
    }
    false
}

#[derive(Clone, Copy)]
enum Mode {
    /// caller (q) vs site candidate (c)
    Query,
    /// two candidates: are they consistent with being one place?
    Peer,
}

impl Matcher {
    pub fn new() -> Matcher {
        Matcher { data: data::Data::deserialize(EMBEDDED), cjk: token::CjkRules::new() }
    }

    /// `text` with no more than `MAX_PARTS` comma parts and about `MAX_CHARS` characters (see
    /// there), and whether the text cut away held a number or a unit word: a row that lost one may
    /// be another unit or building, so it is never matched, only asked about.
    fn bound<'t>(&self, text: &'t str) -> (std::borrow::Cow<'t, str>, bool) {
        use std::borrow::Cow;
        // a Han, kana or Hangul character is read as a word of its own, so it counts twice
        let weight = |c: char| if c >= '\u{2E80}' { 2 } else { 1 };
        let count = |t: &str| t.chars().map(weight).sum::<usize>();
        let parts: Vec<&str> = text.split(',').collect();
        if parts.len() <= MAX_PARTS && count(text) <= MAX_CHARS {
            return (Cow::Borrowed(text), false);
        }
        // a number, or a unit, level, entrance or PO box word
        let telling = |t: &str| {
            t.chars().any(|c| c.is_numeric())
                || data::phrase_key(t).split(' ').any(|w| self.data.lookup(w).iter().any(|e| matches!(e.class, data::Class::UnitNum | data::Class::UnitAlone | data::Class::LevelNum | data::Class::LevelAlone | data::Class::Staircase | data::Class::Entrance | data::Class::PoBox)))
        };
        let n = parts.len();
        let mut keep = vec![true; n];
        let mut len = count(text);
        let mut left = n;
        for i in 1..n.saturating_sub(KEPT_LAST_PARTS) {
            if left <= MAX_PARTS && len <= MAX_CHARS {
                break;
            }
            if !telling(parts[i]) {
                keep[i] = false;
                left -= 1;
                len -= count(parts[i]) + 1;
            }
        }
        let mut kept: Vec<&str> = (0..n).filter(|&i| keep[i]).map(|i| parts[i]).collect();
        let mut lost = false;
        if kept.len() > MAX_PARTS {
            let k = kept.len();
            lost |= kept[KEPT_FIRST_PARTS..k - (MAX_PARTS - KEPT_FIRST_PARTS)].iter().any(|t| telling(t));
            kept = kept[..KEPT_FIRST_PARTS].iter().chain(&kept[k - (MAX_PARTS - KEPT_FIRST_PARTS)..]).copied().collect();
        }
        let t = kept.join(",");
        let chars = count(&t);
        if chars <= MAX_CHARS {
            return (Cow::Owned(t), lost);
        }
        // cut between words where the text has them
        // the byte offset where the first `k` counted characters end
        let at = |k: usize| {
            let mut w = 0;
            t.char_indices().find(|&(_, c)| {
                w += weight(c);
                w > k
            }).map_or(t.len(), |(b, _)| b)
        };
        let gap = |c: char| c.is_whitespace() || c == ',';
        let head_end = at(KEPT_FIRST_CHARS);
        let head = &t[..head_end];
        let head = head.rfind(gap).filter(|&b| b > head_end / 2).map_or(head, |b| &head[..b]);
        let tail_start = at(chars - (MAX_CHARS - KEPT_FIRST_CHARS));
        let tail = &t[tail_start..];
        let tail = tail.find(gap).filter(|&b| b < tail.len() / 2).map_or(tail, |b| &tail[b..]);
        let start = tail.as_ptr() as usize - t.as_ptr() as usize;
        lost |= telling(&t[head.len()..start]);
        (Cow::Owned(format!("{}, {}", head.trim_end_matches(gap), tail.trim_start_matches(gap))), lost)
    }

    /// A match on a row (or a caller's text) that lost a number or unit word to `bound` is asked
    /// about instead, unless another row of the place lost none.
    fn unless_cut(p: Pick, e: Explained, lost: &[bool], query_lost: bool) -> (Pick, Explained) {
        let Pick::Match(i) = p else {
            return (p, e);
        };
        if !query_lost && !lost[i] {
            return (p, e);
        }
        if !query_lost {
            if let Some(&j) = e.same.iter().find(|&&j| !lost[j]) {
                let mut e = e;
                e.index = Some(j);
                return (Pick::Match(j), e);
            }
        }
        let mut rows = e.rows;
        for &j in &e.same {
            if query_lost || lost[j] {
                rows[j].verdict = Verdict::Unknown;
                if !rows[j].reasons.contains(&"place_unconfirmed") {
                    rows[j].reasons.push("place_unconfirmed");
                }
            }
        }
        let same = if e.same.is_empty() { vec![i] } else { e.same };
        (Pick::Ambiguous(same.clone()), Explained::ambiguous(same, vec!["place_unconfirmed"], rows))
    }

    /// A country hint as one of the data's codes: any case, "UK" for the United Kingdom; `None` for
    /// a code the data does not know, which is read as no hint.
    fn country_code(&self, hint: Option<&str>) -> Option<&str> {
        let h = hint?.trim().to_ascii_uppercase();
        let h = if h == "UK" { "GB".to_string() } else { h };
        self.data.countries.get_key_value(h.as_str()).map(|(k, _)| k.as_str())
    }

    fn ctx<'a>(&'a self, hint: Option<&'a str>) -> Ctx<'a> {
        let cc = self.country_code(hint);
        let langs = cc.and_then(|c| self.data.countries.get(c)).map(|c| c.langs.as_slice()).filter(|l| !l.is_empty());
        Ctx { data: &self.data, langs, cc, cjk: &self.cjk, inferred: false }
    }

    /// The country a caller's address and a site's rows are in, read from the texts when the caller
    /// gives none: a country's name (one that is no region's name too, unlike "Georgia"; "UK", "U.K.",
    /// "US", "U.S." or "USA" as the last part), a state or
    /// province with a postcode of its country's form ("IL 62701", "ON K1A 0B1", "NSW 2000"), or a
    /// postcode only one country writes ("K1A 0B1", "SW1A 1AA"). The caller's text
    /// decides when it shows one country (a row that shows another is then another place, see
    /// `row_elsewhere`); when it shows none, the rows
    /// decide when they all show one country and the caller's text names a region of it ("Exampleton,
    /// CA" beside rows in the United States), gives a postcode of its form, or gives no place at all
    /// ("12 Example St"); and a United States state with a row's ZIP code of that state ("Exampleton,
    /// FL" and "Exampleton 32500") shows the United States. Otherwise, or when the texts show two
    /// countries (a region named in full that only another country has counts: "London, Ontario"),
    /// `None`: the texts are read with no country, which asks more often and never guesses (and asks
    /// when rows of two countries would each be the place, see `across_countries`).
    pub fn infer_country(&self, query: &str, rows: &[&str]) -> Option<String> {
        let query = &*self.bound(query).0;
        let rows: Vec<std::borrow::Cow<str>> = rows.iter().map(|r| self.bound(r).0).collect();
        let rows: Vec<&str> = rows.iter().map(|r| &**r).collect();
        let rows = rows.as_slice();
        let q = self.country_evidence(query);
        if q.len() > 1 {
            return None;
        }
        let mut r: Vec<&str> = Vec::new();
        for row in rows {
            for c in self.country_evidence(row) {
                if !r.contains(&c) {
                    r.push(c);
                }
            }
        }
        // the caller's own country: rows that show another are other places (see `row_elsewhere`)
        if let Some(&x) = q.first() {
            return Some(x.to_string());
        }
        let x: &str = match (q.first(), r.as_slice()) {
            (None, [x]) if self.query_in(query, x) || self.no_place(query, x) => x,
            // a state the caller gives and a row's ZIP code of that state ("..., FL" and "... 32500")
            (None, []) if self.query_in(query, "US") && rows.iter().any(|r| self.zip_in_state(query, r)) => "US",
            _ => return None,
        };
        // a region named in full that only other countries have ("London, Ontario" beside "London,
        // United Kingdom") is a second country
        if std::iter::once(query).chain(rows.iter().copied()).any(|t| self.names_region_elsewhere(t, x)) {
            return None;
        }
        Some(x.to_string())
    }

    /// The caller's country: the one given, else the one the caller's own text shows.
    fn home_country(&self, query: &str, country: Option<&str>) -> Option<String> {
        if let Some(c) = country {
            return Some(c.to_string());
        }
        let e = self.country_evidence(query);
        if e.len() == 1 { Some(e[0].to_string()) } else { None }
    }

    /// The row shows a country other than the caller's `cc`, so it is another place: another
    /// country's name, a state or province with its postcode or a postcode form only another country
    /// writes ("London, ON N6A 1A1" for the United Kingdom), a region named in full that only other
    /// countries have and none of `cc`'s, in a country that writes states or
    /// provinces after the town ("London, Ontario"), or, after the town, a state or province
    /// code of a country that writes one there and of no region of `cc` ("Paris, TX" for France).
    /// A country the caller's own text shows is the caller's too (`caller`). A US territory written
    /// as a country of its own is the United States ("Puerto Rico"), and
    /// the United Kingdom is not told apart from Ireland (Northern Ireland is written both ways) or
    /// from the Crown Dependencies, which share its postcode form.
    fn row_elsewhere(&self, row: &str, cc: &str, caller: &[&str]) -> bool {
        let Some(home) = self.data.countries.get(cc) else {
            return false;
        };
        // Northern Ireland addresses are often given as Ireland, with UK postcodes
        let ours = |y: &str| y == cc || caller.contains(&y) || (cc == "US" && matches!(y, "PR" | "GU" | "VI" | "AS" | "MP")) || matches!((cc, y), ("IE" | "JE" | "GG" | "IM", "GB") | ("GB", "IE" | "JE" | "GG" | "IM"));
        let e = self.country_evidence(row);
        if !e.is_empty() {
            // a row that also shows the caller's country is left to the comparison
            return !e.iter().any(|y| ours(y));
        }
        // a postcode of digits in a form the caller's country never writes ("Vancouver 98660" in
        // Canada), in a country whose form is followed (`STRICT_POSTCODE_COUNTRIES`); the United
        // Kingdom's forms include Ireland's and the Crown Dependencies', and a ZIP+4 may lose its
        // hyphen. A postcode with letters is left to `country_evidence`.
        if let (true, Some(p)) = (STRICT_POSTCODE_COUNTRIES.contains(&cc), parse::parse(&self.ctx(None), row).postcode) {
            let group: &[&str] = if cc == "GB" { &["GB", "IE", "JE", "GG", "IM"] } else { std::slice::from_ref(&cc) };
            let fits = |c: &&str| self.data.countries.get(*c).and_then(|co| co.zip()).map_or(false, |form| Self::postcode_as_written_fits(form, row, &p));
            let zip9 = cc == "US" && p.len() == 9 && p.chars().all(|c| c.is_ascii_digit());
            if home.zip().is_some() && p.chars().all(|c| c.is_ascii_digit()) && !group.iter().any(fits) && !zip9 {
                return true;
            }
        }
        let x = parse::parse(&self.ctx(Some(cc)), row);
        // after the town, a state or province named in full where states and provinces follow the
        // town (`POSTAL_CODE_COUNTRIES`), and the row names no region of `cc`
        let ours_named = |k: &str| self.countries_named(&Components { countries: vec![k.to_string()], ..Default::default() }).contains(&cc);
        let state_elsewhere = x.localities.iter().filter(|t| !x.countries.contains(&t.key)).skip(1).filter(|t| t.key.len() >= 4).any(|t| {
            home.region_ids(&t.key).is_empty()
                && !ours_named(&t.key)
                && POSTAL_CODE_COUNTRIES.iter().any(|&o| o != cc && self.data.countries.get(o).map_or(false, |co| co.regions.contains_key(&t.key)))
        });
        if self.region_ids(&x, true).is_empty() && state_elsewhere {
            return true;
        }
        // territories' and military codes ("GU", "AE") are left out: they read as other words
        x.localities.iter().filter(|t| !x.countries.contains(&t.key)).skip(1).any(|t| {
            t.key.len() == 2
                && !matches!(t.key.as_str(), "pr" | "gu" | "vi" | "as" | "mp" | "pw" | "fm" | "mh" | "aa" | "ae" | "ap")
                && home.region_ids(&t.key).is_empty()
                && POSTAL_CODE_COUNTRIES.iter().any(|&o| o != cc && self.data.countries.get(o).map_or(false, |co| co.regions.contains_key(&t.key)))
        })
    }

    /// The postcode `p` (as read: "59300") fits `form` as the text writes it ("59-300") or as read.
    fn postcode_as_written_fits(form: &regex_lite::Regex, text: &str, p: &str) -> bool {
        if form.is_match(p) {
            return true;
        }
        let key = |t: &str| -> String { t.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_ascii_uppercase() };
        let words: Vec<&str> = text.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()).collect();
        (0..words.len()).any(|i| {
            (1..=2usize.min(words.len() - i)).any(|l| {
                let w = words[i..i + l].join(" ");
                let w = w.trim_matches(|c: char| !c.is_alphanumeric());
                key(w) == p.to_ascii_uppercase() && form.is_match(w)
            })
        })
    }

    /// With no country for the caller: the row shows one country, and the caller's postcode is not of
    /// its form ("Toronto 12345-6789" beside "Toronto, ON M5V 1A1").
    fn postcode_elsewhere(&self, query: &str, row: &str) -> bool {
        let e = self.country_evidence(row);
        let [y] = e.as_slice() else {
            return false;
        };
        let Some(form) = self.data.countries.get(*y).and_then(|co| co.zip()) else {
            return false;
        };
        parse::parse(&self.ctx(None), query).postcode.map_or(false, |p| !form.is_match(&p))
    }

    /// The countries the caller's own text shows or names, read with and without the country.
    fn caller_countries(&self, query: &str, cc: &str) -> Vec<&str> {
        let mut v = self.country_evidence(query);
        for c in self.countries_named(&parse::parse(&self.ctx(Some(cc)), query)) {
            if let Some((k, _)) = self.data.countries.get_key_value(c) {
                if !v.contains(&k.as_str()) {
                    v.push(k.as_str());
                }
            }
        }
        v
    }

    /// The row shows the caller's country: its name, or one of its regions.
    fn shows_country(&self, row: &str, cc: &str) -> bool {
        self.country_evidence(row).contains(&cc) || !self.region_ids(&parse::parse(&self.ctx(Some(cc)), row), true).is_empty()
    }

    /// `decide` over the rows that may be in the caller's country; a row that shows another is
    /// another place (`country`).
    fn decide_home(&self, query: &str, rows: &[Row], country: Option<&str>, inferred: bool, ranges: bool) -> (Pick, Explained) {
        let away: Vec<bool> = match self.home_country(query, country) {
            Some(cc) => {
                let caller = self.caller_countries(query, &cc);
                rows.iter().map(|r| self.row_elsewhere(r.text, &cc, &caller)).collect()
            }
            // no country to read by: a row that shows one whose postcodes the caller's does not fit
            None => rows.iter().map(|r| self.postcode_elsewhere(query, r.text)).collect(),
        };
        if !away.contains(&true) {
            let read = self.decide_read(query, rows, country, inferred, ranges);
            if self.home_country(query, country).is_none() {
                if let Some(asked) = self.across_countries(query, rows, ranges, &read.1) {
                    return asked;
                }
            }
            return read;
        }
        let cc = self.home_country(query, country).or_else(|| rows.iter().find_map(|r| self.country_evidence(r.text).first().map(|c| c.to_string()))).unwrap_or_default();
        let kept: Vec<usize> = (0..rows.len()).filter(|&i| !away[i]).collect();
        let sub: Vec<Row> = kept.iter().map(|&i| rows[i]).collect();
        let (p, mut e) = if sub.is_empty() { (Pick::None, Explained::none(vec![])) } else { self.decide_read(query, &sub, country, inferred, ranges) };
        let at = |i: usize| kept[i];
        let p = match p {
            Pick::Match(i) => Pick::Match(at(i)),
            Pick::Ambiguous(v) => Pick::Ambiguous(v.into_iter().map(at).collect()),
            Pick::None => Pick::None,
        };
        e.index = e.index.map(at);
        e.same = e.same.iter().map(|&i| at(i)).collect();
        e.prefer_on_read = e.prefer_on_read.map(at);
        let mut kept_reports = std::mem::take(&mut e.rows).into_iter();
        e.rows = away
            .iter()
            .map(|&a| if a { RowReport { verdict: Verdict::Different, reasons: vec!["country"], notes: vec![] } } else { kept_reports.next().unwrap() })
            .collect();
        // the row taken shows nothing of the caller's country, and a row left out for its country
        // would have been the place too: the caller's address may be in that other country, so ask
        if let Pick::Match(i) = p {
            if !self.shows_country(rows[i].text, &cc) {
                let all = self.decide_read(query, rows, country, inferred, ranges);
                let rival = match &all.0 {
                    Pick::Match(j) => away[*j],
                    Pick::Ambiguous(v) => v.iter().any(|&j| away[j]),
                    Pick::None => false,
                } || all.1.same.iter().any(|&j| away[j]);
                if rival {
                    let reports = e.rows.clone();
                    return (Pick::Ambiguous(vec![i]), Explained::ambiguous(vec![i], vec!["place_unconfirmed"], reports));
                }
            }
        }
        (p, e)
    }

    /// With no country for the caller, the rows show two countries or more (see `row_countries`):
    /// read by each, and when two of them take different rows, the caller's address may be in either
    /// country, so ask ("10 Main St, Vancouver" beside rows in Washington and British Columbia).
    fn across_countries(&self, query: &str, rows: &[Row], ranges: bool, read: &Explained) -> Option<(Pick, Explained)> {
        let mut shown: Vec<&str> = Vec::new();
        for r in rows {
            for c in self.row_countries(r.text) {
                if !shown.contains(&c) {
                    shown.push(c);
                }
            }
        }
        if shown.len() < 2 {
            return None;
        }
        let mut taken: Vec<Vec<usize>> = Vec::new();
        for c in shown.iter().take(4) {
            let ids = match self.decide_home(query, rows, Some(c), true, ranges).0 {
                Pick::Match(i) => vec![i],
                Pick::Ambiguous(v) => v,
                Pick::None => continue,
            };
            if !taken.contains(&ids) {
                taken.push(ids);
            }
        }
        if taken.len() < 2 {
            return None;
        }
        let mut ids: Vec<usize> = taken.concat();
        ids.sort();
        ids.dedup();
        Some((Pick::Ambiguous(ids.clone()), Explained::ambiguous(ids, vec!["several_places"], read.rows.clone())))
    }

    /// The countries a row may be in: those `country_evidence` reads, else, in a row of three parts
    /// or more, those that write a state or province after the town and have one by the row's last
    /// part ("..., Vancouver, WA"). Ireland and the Crown Dependencies count as the United Kingdom,
    /// and the United States' territories as the United States.
    fn row_countries(&self, row: &str) -> Vec<&str> {
        let group = |c: &str| -> String {
            match c {
                "IE" | "JE" | "GG" | "IM" => "GB".to_string(),
                "PR" | "GU" | "VI" | "AS" | "MP" => "US".to_string(),
                _ => c.to_string(),
            }
        };
        let mut v: Vec<String> = self.country_evidence(row).iter().map(|c| group(c)).collect();
        // the last part is a country's own name, and no other country's region ("..., Kingston, Jamaica")
        if v.is_empty() && row.contains(',') {
            let last = data::concat_key(row.rsplit(',').next().unwrap_or(""));
            let named: Vec<&String> = self.data.countries.iter().filter(|(_, co)| last.len() >= 3 && co.names.contains(&last)).map(|(k, _)| k).collect();
            if !self.data.countries.iter().any(|(k, co)| !named.contains(&k) && co.regions.contains_key(&last)) {
                v = named.iter().map(|c| group(c)).collect();
            }
        }
        if v.is_empty() && row.split(',').count() >= 3 {
            let last = fold::ascii_key(&fold::fold(row.rsplit(',').next().unwrap_or("")).chars().filter(|c| c.is_alphanumeric()).collect::<String>());
            v = POSTAL_CODE_COUNTRIES.iter().filter(|&&o| self.data.countries.get(o).map_or(false, |co| co.regions.contains_key(&last))).map(|c| c.to_string()).collect();
        }
        let mut out: Vec<&str> = Vec::new();
        for c in v {
            if let Some((k, _)) = self.data.countries.get_key_value(c.as_str()) {
                if !out.contains(&k.as_str()) {
                    out.push(k.as_str());
                }
            }
        }
        out
    }

    /// The caller gave a state, every row of the place leaves it out (no state, no postcode), and
    /// another row is the same address in another state.
    fn state_left_out_beside_another(&self, q: &Components, cs: &[Components], place: &[usize], verdicts: &[(Verdict, Vec<&'static str>)]) -> bool {
        let states = q.cc.as_deref().map_or(false, |cc| POSTAL_CODE_COUNTRIES.contains(&cc));
        if !states || (self.region_ids(q, false).is_empty() && q.town_region.is_none()) {
            return false;
        }
        let silent = |c: &Components| self.region_ids(c, false).is_empty() && c.town_region.is_none() && c.postcode.is_none();
        if !place.iter().all(|&i| silent(&cs[i])) {
            return false;
        }
        let one = |a: &Components, b: &Components| self.compare_parsed(a, b, Mode::Peer).0 != Verdict::Different && self.compare_parsed(b, a, Mode::Peer).0 != Verdict::Different;
        (0..cs.len()).any(|j| verdicts[j].0 == Verdict::Different && matches!(verdicts[j].1.as_slice(), ["region"] | ["postcode"]) && place.iter().all(|&i| one(&cs[i], &cs[j])))
    }

    /// The text names a country that has no region of the code `k` ("Jakarta ID, Indonesia").
    fn country_without(&self, x: &Components, k: &str) -> bool {
        let keys: Vec<&String> = x.countries.iter().chain(x.localities.iter().map(|t| &t.key)).collect();
        self.data.countries.values().any(|co| keys.iter().any(|t| t.len() >= 3 && co.names.contains(*t)) && !co.regions.contains_key(k))
    }

    /// `a` writes a region after its town with no comma ("Exampleton IL"), and `b` shows only other
    /// regions: in its own part, after its town, or by its ZIP code.
    fn town_region_differs(&self, a: &Components, b: &Components) -> bool {
        let Some(co) = a.cc.as_deref().and_then(|cc| self.data.countries.get(cc)) else {
            // with no country: another code of a country that has the caller's ("IL" and "MO")
            let Some(k) = a.town_region.as_deref() else {
                return false;
            };
            if self.country_without(a, k) || self.country_without(b, k) {
                return false;
            }
            let theirs: Vec<&str> = b.town_region.iter().map(|t| t.as_str()).chain(b.localities.iter().skip(1).map(|t| t.key.as_str())).collect();
            return POSTAL_CODE_COUNTRIES.iter().any(|&cc| {
                let Some(co) = self.data.countries.get(cc) else {
                    return false;
                };
                let ids = co.region_ids(k);
                let mut shown: Vec<u16> = theirs.iter().flat_map(|t| co.region_ids(t)).collect();
                // and, for a United States state, a row's ZIP code read as one
                if cc == "US" && !ids.is_empty() {
                    let mut us = b.clone();
                    us.cc = Some("US".to_string());
                    shown.extend(self.zip_states(&us));
                }
                !ids.is_empty() && !shown.is_empty() && !shown.iter().any(|r| ids.contains(r))
            });
        };
        let Some(ids) = a.town_region.as_deref().map(|k| co.region_ids(k)) else {
            return false;
        };
        let mut theirs: Vec<u16> = self.region_ids(b, false).iter().map(|r| r.0).collect();
        theirs.extend(b.town_region.as_deref().map(|k| co.region_ids(k)).unwrap_or_default());
        theirs.extend(self.zip_states(b));
        !ids.is_empty() && !theirs.is_empty() && !theirs.iter().any(|r| ids.contains(r))
    }

    /// The text names in full (four letters or more) a region of some country and of none in `cc`.
    fn names_region_elsewhere(&self, text: &str, cc: &str) -> bool {
        let Some(home) = self.data.countries.get(cc) else {
            return false;
        };
        let x = parse::parse(&self.ctx(None), text);
        x.localities.iter().filter(|t| t.key.len() >= 4 && !x.countries.contains(&t.key)).any(|t| {
            home.region_ids(&t.key).is_empty() && self.data.countries.iter().any(|(k, co)| k != cc && co.regions.contains_key(&t.key))
        })
    }

    /// The countries a text shows (see `infer_country`).
    fn country_evidence(&self, text: &str) -> Vec<&str> {
        use std::sync::OnceLock;
        static FORMS: OnceLock<[(regex_lite::Regex, &str, &str); 3]> = OnceLock::new();
        // (postcode form, country, whether the form is that country's alone)
        let forms = FORMS.get_or_init(|| {
            [
                (regex_lite::Regex::new(r"^(\d{5})(-\d{4})?$").unwrap(), "US", ""),
                (regex_lite::Regex::new(r"^[a-z]\d[a-z] ?\d[a-z]\d$").unwrap(), "CA", "own"),
                (regex_lite::Regex::new(r"^\d{4}$").unwrap(), "AU", ""),
            ]
        });
        static OWN: OnceLock<[(regex_lite::Regex, &str); 1]> = OnceLock::new();
        let own = OWN.get_or_init(|| [(regex_lite::Regex::new(r"^[a-z]{1,2}\d[a-z\d]? \d[a-z]{2}$").unwrap(), "GB")]);
        let mut v: Vec<&str> = Vec::new();
        let add = |cc: &'static str, v: &mut Vec<&str>| {
            if self.data.countries.contains_key(cc) && !v.contains(&cc) {
                v.push(cc);
            }
        };
        // a country's name, unless another country has a region of the name too ("Georgia"; "England"
        // is a region of the country it names)
        let x = parse::parse(&self.ctx(None), text);
        let folded = fold::fold(text);
        // "UK", "U.K.", "US", "U.S.", "USA" or "MX" as the last part: the usual short names, though
        // "UK" and "US" are region codes elsewhere too
        let parts = folded.split(',').count();
        if parts > 1 {
            let last: String = folded.rsplit(',').next().unwrap_or("").chars().filter(|c| c.is_alphanumeric()).collect();
            match last.as_str() {
                "uk" => add("GB", &mut v),
                "us" | "usa" => add("US", &mut v),
                // no state or province of a country that writes them after the town
                "mx" => add("MX", &mut v),
                _ => {}
            }
        }
        for k in &x.countries {
            // a two-letter code is too short to tell ("Sg" for Sungai); "UK" and "US" are read above
            if k.len() < 3 {
                continue;
            }
            let named = self.countries_named(&Components { countries: vec![k.clone()], ..Default::default() });
            if self.data.countries.iter().any(|(cc, co)| !named.contains(&cc.as_str()) && co.regions.contains_key(k)) {
                continue;
            }
            for cc in named {
                if let Some((key, _)) = self.data.countries.get_key_value(cc) {
                    if !v.contains(&key.as_str()) {
                        v.push(key.as_str());
                    }
                }
            }
        }
        for (si, seg) in folded.split(',').enumerate() {
            // the first of several parts is the street's ("Victoria 1100, Exampleton")
            let street_part = si == 0 && parts > 1;
            let w: Vec<&str> = seg.split_whitespace().map(|t| t.trim_matches(|c: char| !c.is_alphanumeric() && c != '-')).filter(|t| !t.is_empty()).collect();
            for i in 0..w.len() {
                // a postcode of one word, or of two ("K1A 0B1")
                for l in 1..=2usize.min(w.len() - i) {
                    let pc = w[i..i + l].join(" ");
                    for (re, cc) in own.iter() {
                        if re.is_match(&pc) {
                            add(cc, &mut v);
                        }
                    }
                    for (re, cc, alone) in forms.iter() {
                        if !re.is_match(&pc) {
                            continue;
                        }
                        if !alone.is_empty() {
                            add(cc, &mut v);
                        }
                        // a state or province right before it, opening its part ("..., IL 62701") or
                        // written as a code in capitals ("Exampleton IL 62701"); a town's last word
                        // is no state ("Son La 36000"), nor is a word of another script read in Latin letters
                        let Some(co) = self.data.countries.get(*cc) else {
                            continue;
                        };
                        let opens_or_code = |n: usize| {
                            let words = &w[i - n..i];
                            words.iter().all(|x| x.is_ascii()) && (i == n || (words.iter().all(|x| x.len() <= 3) && text.contains(&format!("{} ", words.join(" ").to_uppercase()))))
                        };
                        if !street_part && (1..=3usize.min(i)).any(|n| co.regions.contains_key(&fold::ascii_key(&w[i - n..i].concat())) && opens_or_code(n)) {
                            add(cc, &mut v);
                        }
                    }
                }
            }
        }
        v
    }

    /// The caller's text, read by the country, gives no place: no town, region, postcode or country.
    fn no_place(&self, query: &str, cc: &str) -> bool {
        let x = parse::parse(&self.ctx(Some(cc)), query);
        // a number read as a postcode that is not of the country's form is no place ("County Rd 1800")
        let form = self.data.countries.get(cc).and_then(|co| co.zip());
        let postcode = x.postcode.as_deref().map_or(false, |p| form.map_or(true, |re| re.is_match(p)));
        x.localities.is_empty() && x.regions.is_empty() && x.countries.is_empty() && !postcode && !x.country
    }

    /// The caller's text names a region of the country, or gives a postcode of its form, and
    /// nothing of another country.
    fn query_in(&self, query: &str, cc: &str) -> bool {
        let x = parse::parse(&self.ctx(Some(cc)), query);
        let form = self.data.countries.get(cc).and_then(|co| co.zip());
        let postcode = x.postcode.as_deref().map_or(false, |p| form.map_or(false, |re| re.is_match(p)));
        !self.crosses_hint(&x) && !self.foreign_code(&x) && (!self.region_ids(&x, true).is_empty() || postcode)
    }

    /// The caller's text gives a United States state and the row a ZIP code of that state.
    fn zip_in_state(&self, query: &str, row: &str) -> bool {
        let ctx = self.ctx(Some("US"));
        let (q, c) = (parse::parse(&ctx, query), parse::parse(&ctx, row));
        let zone = self.zip_states(&c);
        q.postcode.is_none() && self.region_ids(&q, false).iter().any(|r| zone.contains(&r.0)) && self.region_ids(&c, false).is_empty()
    }

    /// The caller's units as the caller wrote them, joined with ", ".
    fn unit_as_written(&self, query: &str, country: Option<&str>) -> Option<String> {
        let x = parse::parse(&self.ctx(country), query);
        let alnum = |t: &str| -> String { fold::fold(t).chars().filter(|c| c.is_alphanumeric()).collect() };
        // the query's words with their byte ranges
        let mut words: Vec<(usize, usize)> = Vec::new();
        let mut start = None;
        for (i, ch) in query.char_indices() {
            if ch.is_whitespace() || ch == ',' {
                if let Some(s0) = start.take() {
                    words.push((s0, i));
                }
            } else if start.is_none() {
                start = Some(i);
            }
        }
        if let Some(s0) = start {
            words.push((s0, query.len()));
        }
        let mut out: Vec<String> = Vec::new();
        for u in &x.unit_words {
            let want = alnum(u);
            if want.is_empty() {
                continue;
            }
            'found: for i in 0..words.len() {
                let mut got = String::new();
                for j in i..words.len() {
                    got.push_str(&alnum(&query[words[j].0..words[j].1]));
                    if got == want {
                        let t = query[words[i].0..words[j].1].trim_matches(|c: char| c == '.' || c == ';' || c == ':').to_string();
                        if !out.contains(&t) {
                            out.push(t);
                        }
                        break 'found;
                    }
                    if got.len() >= want.len() || !want.starts_with(&got) {
                        break;
                    }
                }
            }
        }
        if out.is_empty() { None } else { Some(out.join(", ")) }
    }

    /// Parse an address into the components used for matching.
    pub fn normalize(&self, text: &str, country: Option<&str>) -> Components {
        parse::parse(&self.ctx(country), text)
    }

    /// Is the candidate the same place as the query? Returns the verdict and the reasons.
    pub fn compare(&self, query: &str, candidate: &str, country: Option<&str>) -> (Verdict, Vec<&'static str>) {
        let ((query, ql), (candidate, cl)) = (self.bound(query), self.bound(candidate));
        match self.compare_whole(&query, &candidate, country) {
            // a text that lost a number or unit word may be another unit or building
            (Verdict::Same, _) if ql || cl => (Verdict::Unknown, vec!["place_unconfirmed"]),
            v => v,
        }
    }

    /// `compare` on texts within `MAX_PARTS` and `MAX_CHARS`.
    fn compare_whole(&self, query: &str, candidate: &str, country: Option<&str>) -> (Verdict, Vec<&'static str>) {
        let country = self.country_code(country);
        // a site's control ("See more results for <text>") is never a place
        if self.is_control(query, candidate) {
            return (Verdict::Different, vec!["control"]);
        }
        let inferred = if country.is_none() { self.infer_country(query, &[candidate]) } else { None };
        let away = match self.home_country(query, country.or(inferred.as_deref())) {
            Some(cc) => self.row_elsewhere(candidate, &cc, &self.caller_countries(query, &cc)),
            None => self.postcode_elsewhere(query, candidate),
        };
        if away {
            return (Verdict::Different, vec!["country"]);
        }
        let mut ctx = self.ctx(country.or(inferred.as_deref()));
        ctx.inferred = inferred.is_some() && ctx.cc.is_some();
        let q = parse::parse(&ctx, query);
        if let Some(name) = self.place_name(query, &q) {
            return match self.named_decide(&ctx, query, &name, &[Row::from(candidate)]).0 {
                Pick::Match(_) => (Verdict::Same, vec![]),
                Pick::Ambiguous(_) => (Verdict::Unknown, vec!["place_name_variant"]),
                Pick::None => (Verdict::Different, vec!["place_name"]),
            };
        }
        // a listing that opens with the distance to it is compared without it
        let candidate = Self::without_distance(candidate);
        let c = self.parse_row(&ctx, query, &q, &candidate);
        let alts: Vec<Components> = Self::comma_free_readings(query).iter().map(|a| parse::parse(&ctx, a)).collect();
        let calt = self.without_leading_name(&candidate).map(|(t, name)| (parse::parse(&ctx, &t), name));
        self.compare_any(query, &q, &alts, &c, calt.as_ref())
    }

    /// A listing that starts with a business or building name holding digits ("Grand 2009 Ltd,
    /// Main St 6, Town"): the listing without that first part, so its digits are not read as the
    /// house number.
    /// It also returns the name part dropped, if any. A unit, floor or building part is never
    /// dropped.
    fn without_leading_name(&self, candidate: &str) -> Option<(String, Option<String>)> {
        let parts: Vec<&str> = candidate.split(',').collect();
        // a unit, floor or building part is part of the address, not a name
        if parts.len() < 3 || !parts[0].chars().any(|ch| ch.is_numeric()) || !parts[1].chars().any(|ch| ch.is_numeric()) || Self::unit_part(parts[0]) {
            return None;
        }
        let rest = parts[1..].join(",");
        // a listing that repeats its address ("Example Rd 16C, Example Rd 16C, …"), or names a place
        // after it ("12 Example Rd Parking, 12 Example Rd, …"), drops no other address
        let worded = parts[0].chars().any(|ch| ch.is_alphabetic());
        let repeats = Self::holds_part(&rest, parts[0]) || self.address_then_name(parts[0], parts[1]);
        let name = (!worded || !repeats).then(|| parts[0].to_string());
        Some((rest, name))
    }

    /// A first part that is the listing's next part, its address on a named street, with a place's
    /// name after it ("12 Example Rd Parking" before "12 Example Rd, …"): the words after the address
    /// hold no number and no word an address uses (street type, direction, unit, level, entrance,
    /// staircase, house-number word or PO box), so the part names a place at that address, not
    /// another address or a unit of it. A next part that is only a number ("12 AB, 12, Example St")
    /// names no street, so its first part may be a unit.
    fn address_then_name(&self, first: &str, address: &str) -> bool {
        use data::Class::*;
        let (first, address) = (data::phrase_key(first), data::phrase_key(address));
        let Some(after) = first.strip_prefix(&address).and_then(|t| t.strip_prefix(' ')) else {
            return false;
        };
        let address_word = |w: &str| {
            w.chars().any(|ch| ch.is_numeric())
                || self.data.lookup(w).iter().any(|e| matches!(e.class, StreetType | Directional | UnitNum | UnitAlone | LevelNum | LevelAlone | Entrance | Staircase | PoBox | HouseKw))
        };
        address.chars().any(|ch| ch.is_alphabetic()) && after.split(' ').any(|w| w.chars().count() >= 2) && !after.split(' ').any(address_word)
    }

    /// A part that names a unit, floor or building within an address rather than a place: "#7",
    /// "Unit 7", "Apt 7B", "Suite 310", "Bldg 7", "7th Floor", "Level 7".
    fn unit_part(part: &str) -> bool {
        const WORDS: &[&str] = &[
            "unit", "apt", "apartment", "suite", "ste", "building", "bldg", "bld", "floor", "fl", "flr", "flat", "lot", "space", "spc", "room", "rm", "shop", "level", "lvl", "office", "ofc", "dept", "trailer", "trlr", "stall", "slip", "pier", "hangar", "box",
        ];
        let t = part.trim();
        if t.starts_with('#') {
            return true;
        }
        let key = data::phrase_key(t);
        let w: Vec<&str> = key.split(' ').filter(|w| !w.is_empty()).collect();
        let num = |i: usize| w.get(i).map_or(false, |x| x.chars().any(|ch| ch.is_ascii_digit()));
        let word = |i: usize| w.get(i).map_or(false, |x| WORDS.contains(x) || *x == "#");
        // a unit word and its number ("Unit 7", "Apt # 7"), or a number standing alone and its unit
        // word ("7th Floor", not "Sample9 Room")
        let raw: Vec<&str> = t.split_whitespace().collect();
        let number_then_word = raw.len() == 2 && Self::unit_number(raw[0]).is_some() && word(w.len().saturating_sub(1));
        (word(0) && (num(1) || (matches!(w.get(1), Some(&"#") | Some(&"no")) && num(2)))) || number_then_word
    }

    /// A number as a unit may be written: digits, then at most two letters or "/F" ("7", "12B",
    /// "7F", "7th", "7/F"). Lowercased.
    fn unit_number(token: &str) -> Option<String> {
        let t = token.trim_end_matches(['.', ':', ')']);
        let digits = t.len() - t.trim_start_matches(|ch: char| ch.is_numeric()).len();
        if digits == 0 {
            return None;
        }
        let rest = &t[digits..];
        let rest = rest.strip_prefix('/').unwrap_or(rest);
        (rest.chars().count() <= 2 && rest.chars().all(|ch| ch.is_alphabetic())).then(|| t.to_lowercase())
    }

    /// A part whose number is the listing's postcode (a postal branch, "Sample Post 60000") names no
    /// unit.
    fn postcode_number(part: &str, c: &Components) -> bool {
        let pc: String = c.postcode.as_deref().unwrap_or("").chars().filter(|ch| ch.is_alphanumeric()).collect::<String>().to_lowercase();
        // (with a name: a number and the postcode alone may still be a unit)
        let named = part.split(|ch: char| !ch.is_alphanumeric()).any(|w| w.chars().count() >= 2 && w.chars().all(|ch| ch.is_alphabetic()));
        named && pc.len() >= 3 && part.split(|ch: char| !ch.is_alphanumeric()).any(|w| w.to_lowercase() == pc)
    }

    /// The caller wrote the part too ("Sample 57", "11, 12 Example Ave"), other than as the house
    /// number or postcode alone ("7" for a caller at 7 Example St) inside another part.
    fn caller_wrote(query: &str, q: &Components, part: &str) -> bool {
        let key = |t: &str| t.chars().filter(|ch| ch.is_alphanumeric()).collect::<String>().to_lowercase();
        let own = [q.house.as_deref(), q.postcode.as_deref()].iter().flatten().any(|h| key(part) == key(h));
        // a part of its own in the caller's text counts, even a number ("306, 11 Example Rd"), but
        // the caller's house number standing alone ("7, Example St") only when the caller wrote it
        // again elsewhere ("60, Example Rd 60")
        let house_alone = q.house.as_deref().map_or(false, |h| key(part) == key(h));
        let twice = q.house.as_deref().map_or(false, |h| query.split(',').filter(|p| p.split(|ch: char| !ch.is_alphanumeric()).any(|w| !w.is_empty() && key(w) == key(h))).count() >= 2);
        let own_part = (!house_alone || twice) && query.split(',').any(|p| !key(p).is_empty() && key(p) == key(part));
        own_part || (!own && Self::holds_part(query, part))
    }

    /// A part that reads as a place's name rather than a unit: two words or more, with the number
    /// not at the end ("24 Hour Sample", "7 Sample Store"). A unit puts its number last ("Villa No
    /// 7", "Lower Level 7") or has a single word ("Studio 12", "7e étage"), and a street address of
    /// its own ("50 Example Street", "50A Example Street", "No. 50 Example Street") is no name
    /// either. Words after a unit's number ("Block 7 East", "Piso 3 Puerta B") look the same, so
    /// the part is a name only where it cannot be another unit than the caller's: for a caller
    /// with a unit, every number in it is that unit's ("24 Hour Sample" for Suite 24); for a caller
    /// with none, it opens with its number and names no side ("24 Hour Sample", not "Tower 2 West"
    /// or "7 East Wing").
    fn named_part(&self, part: &str, q: &Components) -> bool {
        let toks: Vec<&str> = part.split_whitespace().map(|t| t.trim_matches(|ch: char| !ch.is_alphanumeric())).filter(|t| !t.is_empty()).collect();
        let words = toks.iter().filter(|t| t.chars().count() >= 2 && t.chars().all(|ch| ch.is_alphabetic())).count();
        let last_number = toks.last().map_or(true, |t| t.chars().any(|ch| ch.is_numeric()));
        let key = data::phrase_key(part);
        let typed = key.split(' ').any(|w| self.data.lookup(w).iter().any(|e| e.class == data::Class::StreetType));
        // a house number: digits with letters or a range ("50A", "50-52"), or "No." and a number
        let house_number = |t: &str| t.starts_with(|ch: char| ch.is_numeric()) && t.chars().all(|ch| ch.is_alphanumeric() || ch == '-' || ch == '/');
        let numbered = toks.first().map_or(false, |t| house_number(t))
            || (toks.len() >= 2 && matches!(toks[0].to_lowercase().as_str(), "no" | "nr" | "nº" | "n") && toks[1].starts_with(|ch: char| ch.is_numeric()));
        let address = typed && numbered;
        let numbers = |t: &str| t.split(|ch: char| !ch.is_numeric()).filter(|d| !d.is_empty()).map(str::to_string).collect::<Vec<String>>();
        let fits = if q.units.is_empty() {
            // (a direction after the number is a side of the building: "7 East Wing")
            let side = key.split(' ').any(|w| self.data.lookup(w).iter().any(|e| e.class == data::Class::Directional));
            toks.first().map_or(false, |t| t.chars().all(|ch| ch.is_numeric())) && !side
        } else {
            let units: Vec<String> = q.units.iter().flat_map(|u| numbers(u)).collect();
            let own = numbers(part);
            !own.is_empty() && own.iter().all(|d| units.contains(d))
        };
        words >= 2 && !last_number && !address && fits
    }

    /// The text holds the part's words in a row.
    fn holds_part(text: &str, part: &str) -> bool {
        let p = data::phrase_key(part);
        !p.is_empty() && format!(" {} ", data::phrase_key(text)).contains(&format!(" {} ", p))
    }

    /// A short part that may name a unit whatever its word ("№7", "No. 7", "Studio 12", "U7",
    /// "7F", "12B", "7", "7-B"): a number alone or with a letter, a symbol and a number, one word
    /// and a number, or one or two letters joined to a number. It returns the number. A longer name ("12 Hour Sample",
    /// "Studio 12 Hair") is not one.
    fn maybe_unit(part: &str) -> Option<String> {
        let part = fold::fold(part);
        let raw: Vec<&str> = part.split_whitespace().collect();
        let symbol = |s: &str| !s.is_empty() && s.chars().all(|ch| !ch.is_alphanumeric());
        let word = |s: &str| {
            let s = s.trim_end_matches('.');
            !s.is_empty() && s.chars().all(|ch| ch.is_alphabetic())
        };
        let letters = |s: &str| !s.is_empty() && s.chars().count() <= 2 && s.chars().all(|ch| ch.is_alphabetic());
        match raw.as_slice() {
            [one] => {
                if let Some(n) = Self::unit_number(one) {
                    return Some(n);
                }
                // "7-B", "U-7"
                if let Some((a, b)) = one.split_once('-') {
                    if letters(b) || letters(a) {
                        let n = if letters(b) { Self::unit_number(a) } else { Self::unit_number(b).filter(|n| n.chars().all(|ch| ch.is_numeric())) };
                        return n.map(|n| if letters(b) { format!("{}{}", n, b.to_lowercase()) } else { n });
                    }
                }
                // "#7", "№7", "U7", "PH12"
                let lead: String = one.chars().take_while(|ch| !ch.is_numeric()).collect();
                let n = Self::unit_number(&one[lead.len()..])?;
                (symbol(&lead) || (word(&lead) && lead.chars().count() <= 2)).then_some(n)
            }
            // "7 B"
            [a, b] if letters(b) && Self::unit_number(a).map_or(false, |n| n.chars().all(|ch| ch.is_numeric())) => Some(format!("{}{}", a, b.to_lowercase())),
            [a, b] if symbol(a) || word(a) => Self::unit_number(b),
            _ => None,
        }
    }

    /// A dropped leading part that is a place's name holding the caller's house number ("12 Hour
    /// Sample", "Sample79 Mall"), not a street address of its own ("12 Example Rd").
    fn numbered_name(&self, name: &str, house: &str) -> bool {
        let key = data::phrase_key(name);
        if !key.split(' ').any(|w| w.eq_ignore_ascii_case(house)) {
            return false;
        }
        // an address: the number standing alone at either end, and a street type
        let raw: Vec<&str> = name.split_whitespace().collect();
        let alone = |w: Option<&&str>| w.map_or(false, |w| w.trim_matches(|ch: char| !ch.is_alphanumeric()).eq_ignore_ascii_case(house));
        let typed = key.split(' ').any(|w| self.data.lookup(w).iter().any(|e| e.class == data::Class::StreetType));
        !((alone(raw.first()) || alone(raw.last())) && typed)
    }

    /// A listing read for the query `q`: as written, or, when the caller gave only a postal code and
    /// the listing opens with that code before a town ("62701 Exampleton, IL, USA", "75001
    /// Exampleville, France", as map services list a postal code's area), as that town under that
    /// code (`postcode_area`).
    fn parse_row(&self, ctx: &Ctx, query: &str, q: &Components, candidate: &str) -> Components {
        self.postcode_area(ctx, query, q, candidate).unwrap_or_else(|| parse::parse(ctx, candidate))
    }

    /// A listing that opens with the postal code a caller gave alone, as written, and then names
    /// only a town, region or country: that area under the code, not a house number on a street
    /// named for the town. A listing that names a street, a unit, a PO box or another postal code
    /// after the number ("62701 Example Street, Exampleton"), or a town after its first part
    /// ("62701, Example Trail, Sampleton, IL"), is read as written, and so is every listing for a
    /// caller who gave more than a postal code.
    fn postcode_area(&self, ctx: &Ctx, query: &str, q: &Components, candidate: &str) -> Option<Components> {
        let code = q.postcode.as_ref()?;
        let only_code = q.house.is_none() && q.street.is_empty() && q.city.is_none() && q.localities.is_empty() && q.regions.is_empty() && q.units.is_empty() && q.po_box.is_none() && q.business.is_empty();
        let written = query.trim();
        if !only_code || written.is_empty() || !written.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == ' ' || ch == '-') {
            return None;
        }
        let listed = candidate.trim_start();
        let head = listed.get(..written.len()).filter(|h| h.eq_ignore_ascii_case(written))?;
        let rest = &listed[head.len()..];
        // the code ends at a space or a comma: "627010 Exampleton" opens with another number
        if !rest.is_empty() && !rest.starts_with([' ', ',']) {
            return None;
        }
        let area_text = rest.trim_start_matches([' ', ',']);
        let town_part = area_text.split(',').next().unwrap_or("");
        let own_part = rest.trim_start().starts_with(',');
        // the parts after the code read as if a street came first, so the first is a place
        // (`area_parse`), as the code's area would be
        let mut shape = parse::parse(ctx, &format!("1 Example St, {}", area_text));
        // only a region, a country or this code may follow the town's part: a town after it makes
        // that part a street ("62701, Example Trail, Sampleton, IL"; "62701 Example, Sampleton, IL")
        // (a county is no town: "62701, Exampleton, Example County, IL")
        let county = |t: &Text| t.toks.len() >= 2 && t.toks.last().map_or(false, |w| matches!(w.0.as_str(), "county" | "parish" | "borough"));
        let places: Vec<&Text> = shape.localities.iter().filter(|t| !shape.countries.contains(&t.key) && !county(t)).collect();
        let towns = places.iter().filter(|t| !shape.regions.contains(&t.key)).count();
        if places.len() > 2 || towns > 1 {
            return None;
        }
        // the code as a part of its own ("62701, Example Grove, IL, USA"): every part after it is
        // its area, a town named like a street included; a first part that holds a number is no town
        let first_numbered = town_part.chars().any(|ch| ch.is_numeric());
        // the code written right before its town with no comma ("92999 Example Grove, CA, USA"): the
        // part is the town unless it reads as a street, then maybe its town: a street type in it
        // other than `TOWN_TYPES` ("92999 Example Road", "92999 Example Loop Spring Valley"), or a
        // type word with a word that is no type word after it ("92999 Example Grove Sampleton");
        // town names end in place-like types ("Example Grove", "Example Garden Grove", "Spring Valley")
        let typed_inside = || {
            let towns = town_type_keys();
            // per word: whether it is a street type, and whether it is one that makes the part a
            // street (a lone ASCII letter never does: postal districts end in one, "8000 Aarhus C")
            let words: Vec<(bool, bool)> = data::phrase_key(town_part)
                .split(' ')
                .filter(|w| !w.is_empty())
                .map(|w| {
                    let mut types = self.data.lookup(w).iter().filter(|e| e.class == data::Class::StreetType).map(|e| &self.data.canon_keys[e.canon as usize]).peekable();
                    let typed = types.peek().is_some();
                    (typed, typed && (w.chars().count() > 1 || !w.is_ascii()) && types.any(|k| !towns.contains(k)))
                })
                .collect();
            let first_typed = words.iter().position(|w| w.0);
            // a type word with a plain word after it ("Example Grove Sampleton"); or a street, then a
            // town named only with place-like types: a name then three or more of them ("Example
            // Grove Spring Valley"), as a town's name ends in at most two after a name ("Palm Beach
            // Gardens"), or four or more of them alone ("Spring Grove Spring Valley"), as a town
            // named only with them has at most three ("Bay Harbor Islands")
            let all_types = |i: usize| words.len() - i >= 3 && (i > 0 || words.len() >= 4);
            words.iter().any(|w| w.1) || first_typed.map_or(false, |i| words[i + 1..].iter().any(|w| !w.0) || all_types(i))
        };
        let as_town = !first_numbered && (own_part || !typed_inside());
        let mut area = if as_town {
            shape.house = None;
            shape.street = Text::default();
            shape.street_type.clear();
            shape
        } else {
            parse::parse(ctx, area_text)
        };
        let names_area = area.city.is_some() || !area.localities.is_empty() || !area.regions.is_empty();
        let names_more = area.house.is_some() || !area.street.is_empty() || !area.units.is_empty() || area.po_box.is_some() || !area.business.is_empty() || area.second_number.is_some();
        if !names_area || names_more || area.postcode.as_ref().map_or(false, |p| p != code) {
            return None;
        }
        area.postcode = Some(code.clone());
        // the code's own part, or the street part read in its place
        area.parts += usize::from(own_part && first_numbered);
        area.parts -= usize::from(as_town && !own_part && area.parts > 1);
        Some(area)
    }

    /// The listing as compared: without the distance it may open with ("0.2 mi, Sample Cafe, 12
    /// Example St"), so the distance is never read as its house number.
    fn without_distance(candidate: &str) -> std::borrow::Cow<'_, str> {
        Self::without_leading_distance(candidate).map_or(std::borrow::Cow::Borrowed(candidate), std::borrow::Cow::Owned)
    }

    /// A listing that opens with a distance ("0.2 mi", "1.4 km", "350 ft", "0,3 km") and goes on:
    /// the rest of it.
    fn without_leading_distance(candidate: &str) -> Option<String> {
        let t = candidate.trim_start();
        let digits = |s: &str| s.len() - s.trim_start_matches(|ch: char| ch.is_ascii_digit()).len();
        let mut i = digits(t);
        if i == 0 {
            return None;
        }
        // a decimal part, with a point or a comma
        let mut decimal = false;
        if let Some(sep) = t[i..].chars().next().filter(|ch| *ch == '.' || *ch == ',') {
            let n = digits(&t[i + sep.len_utf8()..]);
            if n > 0 {
                i += sep.len_utf8() + n;
                decimal = true;
            }
        }
        let rest = t[i..].trim_start();
        let unit_len = ["miles", "mile", "mi", "km", "ft", "yd", "m"]
            .iter()
            .find(|u| rest.get(..u.len()).map_or(false, |h| h.eq_ignore_ascii_case(u)) && rest.get(u.len()..).map_or(false, |t| t.trim_start().starts_with(',')))?
            .len();
        let after = rest[unit_len..].trim_start().strip_prefix(',')?.trim_start();
        // a whole number right before an address part is a unit ("5M, 350 Example Ave", "5 Ft, …"),
        // not a distance: a distance has a decimal or a name after it
        if !decimal && after.starts_with(|ch: char| ch.is_numeric()) {
            return None;
        }
        (after.split(',').count() >= 2).then(|| after.to_string())
    }

    /// The query as read, or as one of its comma-free readings if that reading is the candidate's
    /// place.
    fn compare_any(&self, query: &str, q: &Components, alts: &[Components], c: &Components, calt: Option<&(Components, Option<String>)>) -> (Verdict, Vec<&'static str>) {
        let first = self.compare_parsed(q, c, Mode::Query);
        // a short leading part that may be a unit ("№7", "Studio 12", "7F") before the caller's
        // number is read as the listing's unit: another unit is not the caller's, and the caller's
        // own number or no unit at all is at most asked about. Not when the caller wrote that part
        // ("Sample 57, 12 Example St") or its word ("Example Rd 76" for a caller on Example Rd).
        let callers_word = |name: &str| {
            let caller_words = format!(" {} ", data::phrase_key(query));
            data::phrase_key(name).split(' ').any(|w| w.chars().count() >= 3 && !w.chars().any(|ch| ch.is_numeric()) && caller_words.contains(&format!(" {} ", w)))
        };
        let lead_unit = calt.and_then(|(_, name)| name.as_deref().filter(|n| !Self::caller_wrote(query, q, n)).and_then(Self::maybe_unit));
        if let (Some((c2, Some(name))), Some(n)) = (calt, &lead_unit) {
            if q.house.is_some() && c2.house == q.house && c2.units.is_empty() && !c2.street.key.is_empty() && !callers_word(name) && !Self::postcode_number(name, c2) {
                // (the rest must be the caller's place first: another town is no unit to ask about)
                let base = self.compare_parsed(q, c2, Mode::Query);
                if base.0 != Verdict::Same {
                    return if first.0 == Verdict::Same { first } else { base };
                }
                let mut c3 = c2.clone();
                c3.units.push(format!("u{}", n));
                let r = self.compare_parsed(q, &c3, Mode::Query);
                return if r.0 == Verdict::Same || r.1 == ["candidate_adds_unit"] { (Verdict::Different, vec!["leading_unit"]) } else { r };
            }
        }
        if first.0 == Verdict::Same {
            return first;
        }
        if let (Some((c2, name)), None) = (calt, &lead_unit) {
            // the listing read without its leading name (never a unit part, see
            // `without_leading_name`) gives another number, or the name holds the caller's own number
            // and is no street address of its own ("12 Hour Sample, 12 Example St"), or only a
            // distance was dropped
            let numbered_name = |n: &String| q.house.as_deref().map_or(false, |h| self.numbered_name(n, h));
            if q.house.is_some() && c2.house == q.house && (c2.house != c.house || name.as_ref().map_or(true, numbered_name)) {
                let r = self.compare_parsed(q, c2, Mode::Query);
                if r.0 == Verdict::Same {
                    // a dropped part that does not hold the caller's number may be a unit of any
                    // form ("7, 12 Example St", "U-7", "Lower Level 7"): another number than the
                    // address's is at most asked about for a caller with a unit, and a short part
                    // is asked about for a caller with none. A row with the caller's own unit still
                    // wins beside it.
                    if let Some(n) = name.as_ref().filter(|n| c2.house != c.house && c2.units.is_empty() && !numbered_name(n) && !self.named_part(n, q) && !Self::postcode_number(n, c2) && !Self::caller_wrote(query, q, n)) {
                        let digits = |t: &str| t.split(|ch: char| !ch.is_numeric()).filter(|d| !d.is_empty()).map(str::to_string).collect::<Vec<String>>();
                        let house: String = c2.house.as_deref().map(|h| h.chars().filter(|ch| ch.is_numeric()).collect()).unwrap_or_default();
                        let other = digits(n).iter().any(|d| *d != house);
                        let short = n.split_whitespace().count() <= 3;
                        if (!q.units.is_empty() && other) || (q.units.is_empty() && short) {
                            return (Verdict::Different, vec!["leading_unit"]);
                        }
                    }
                    return r;
                }
            }
        }
        for a in alts {
            // a reading counts only when it takes another number as the house number, keeps a
            // street and loses no postcode
            if a.house == q.house || a.street.is_empty() || a.postcode != q.postcode {
                continue;
            }
            let r = self.compare_parsed(a, c, Mode::Query);
            if r.0 == Verdict::Same {
                return r;
            }
        }
        first
    }

    /// An address written without commas may end its street part after any number in it
    /// ("1244. Sokak 6 Karşıyaka" is street "1244. Sokak", number 6, town "Karşıyaka"): the readings
    /// with a comma after each number but the last word.
    fn comma_free_readings(query: &str) -> Vec<String> {
        if query.contains([',', ';', '\n', '،', '、', '|']) {
            return Vec::new();
        }
        let words: Vec<&str> = query.split_whitespace().collect();
        let nums: Vec<usize> = (0..words.len()).filter(|&i| words[i].chars().any(|ch| ch.is_numeric())).collect();
        if nums.len() < 2 {
            return Vec::new();
        }
        nums.iter()
            .filter(|&&i| i >= 1 && i + 1 < words.len())
            .take(4)
            .map(|&i| format!("{}, {}", words[..=i].join(" "), words[i + 1..].join(" ")))
            .collect()
    }

    fn compare_parsed(&self, q: &Components, c: &Components, mode: Mode) -> (Verdict, Vec<&'static str>) {
        let r = self.compare_parts(q, c, mode);
        if r.0 == Verdict::Same && self.city_reading_unconfirmed(q, c) {
            return (Verdict::Unknown, vec!["city_reading"]);
        }
        if r.0 == Verdict::Same && matches!(mode, Mode::Query) {
            if let Some(why) = self.area_unconfirmed(q, c) {
                return (Verdict::Unknown, vec![why]);
            }
        }
        r
    }

    /// A row that leaves out where the caller's place is cannot confirm it: the caller names a town
    /// and the row none ("12 Example St, USA", "Exampleton Mall, 1 Example St"), or, where towns
    /// share names across states (`POSTAL_CODE_COUNTRIES`), the caller gives a state or postcode
    /// and the row shows a postcode or country but no state ("1 Example St, Exampleton 60000, USA"
    /// for "..., Exampleton, IL 60100"), or leaves out both the state and the caller's postcode;
    /// a row that leaves out only the state is taken; with no
    /// country to tell a town from a state, the caller gives a place, or a postcode, the row does
    /// not. The same postcode on both sides confirms it.
    fn area_unconfirmed(&self, q: &Components, c: &Components) -> Option<&'static str> {
        if q.postcode.is_some() && q.postcode == c.postcode {
            return None;
        }
        // a lone compass word read as a town ("1 Example St NW") is not a town
        let towns = |x: &Components| -> usize { self.towns(x).iter().filter(|t| !(t.toks.len() == 1 && !directions(t).is_empty())).count() };
        // a row written without commas may keep its town at the end of the street part
        let in_street = |x: &Components| self.towns(q).iter().any(|t| t.key.len() >= 3 && x.street.key.len() > t.key.len() && x.street.key.ends_with(&t.key));
        let places = |x: &Components| x.localities.iter().filter(|t| !x.countries.contains(&t.key) && !(t.toks.len() == 1 && !directions(t).is_empty())).count();
        if towns(q) > 0 && places(c) == 0 && !in_street(c) {
            return Some("town_not_listed");
        }
        let states = q.cc.as_deref().map_or(false, |cc| POSTAL_CODE_COUNTRIES.contains(&cc));
        // a ZIP code of the caller's state confirms the state ("Exampleton 93600" for "Exampleton,
        // CA"); one of another state is another place, told apart before this
        let town_region_ids: Vec<u16> = q.town_region.as_deref().and_then(|k| q.cc.as_deref().and_then(|cc| self.data.countries.get(cc)).map(|co| co.region_ids(k))).unwrap_or_default();
        let zip_confirms = q.postcode.is_none() && (self.region_ids(q, false).iter().any(|r| self.zip_states(c).contains(&r.0)) || town_region_ids.iter().any(|r| self.zip_states(c).contains(r)));
        // a state written after the town with no comma ("Exampleton IL") is given too
        let town_region = q.town_region.is_some() && c.town_region != q.town_region;
        // a row that leaves out only the state, and shows no postcode or country, is taken: nothing
        // in it contradicts the caller (a same-named town in another state is the risk left)
        let own_country = |x: &Components| self.countries_named(x).iter().all(|k| Some(*k) == q.cc.as_deref());
        // Its town is in a part of its own, or follows the caller's street word for word with no
        // comma ("1 Example St Exampleton"; in "1 Example St St Exampleton", "St" may begin the
        // town). A caller's postcode the row leaves out too is a second part of the address it
        // does not confirm, a place's listing by name, street and town included.
        // (the caller's text may run its town on into its street too: "1 Example St Exampleton, IL")
        let runs = |x: &Components| -> [String; 2] {
            let town = x.city.as_ref().filter(|t| !x.regions.contains(&t.key)).map_or("", |t| t.key.as_str());
            let word = if x.street_type_first { "" } else { x.street_type_word.as_str() };
            [format!("{}{}{}", x.street.key, word, town), format!("{}{}", x.street.key, town)]
        };
        let (rc, rq) = (runs(c), runs(q));
        let run_on = rc[0] == rq[0] || rc[1] == rq[1];
        let own_part = c.parts >= 2 || (!q.street.key.is_empty() && ((c.street.key == q.street.key && c.street_type == q.street_type) || run_on));
        let postcode_too = q.postcode.is_some();
        let row_silent = towns(c) > 0 && own_part && !postcode_too && c.postcode.is_none() && own_country(c) && c.town_region.is_none();
        if states && !row_silent && (!self.region_ids(q, false).is_empty() || q.postcode.is_some() || town_region) && self.region_ids(c, false).is_empty() && !zip_confirms && !(c.town_region.is_some() && c.town_region == q.town_region) {
            return Some("region_not_listed");
        }
        let shown = |t: &Text| c.localities.iter().any(|u| u.key == t.key || text_eq(self, t, u, false) == Tri::Yes) || in_street(c);
        // with no country, a row with no postcode cannot confirm the caller's ("Toronto 12345-6789"
        // against "Toronto")
        if q.cc.is_none() && q.postcode.is_some() && c.postcode.is_none() {
            return Some("region_not_listed");
        }
        if q.cc.is_none() && q.localities.iter().any(|t| !q.countries.contains(&t.key) && !shown(t)) {
            return Some("region_not_listed");
        }
        // with no country, a state code after the caller's town ("Exampleton IL") that the row does
        // not show
        if let (None, Some(k)) = (q.cc.as_deref(), q.town_region.as_deref()) {
            // unless either text shows a country with no state of the code ("Berlin DE, Germany")
            let elsewhere = self.country_without(q, k) || self.country_without(c, k);
            if !elsewhere && c.town_region.as_deref() != Some(k) && !c.localities.iter().any(|t| t.key == k) {
                return Some("region_not_listed");
            }
        }
        None
    }

    /// `compare_parsed` before a town read across scripts is confirmed.
    fn compare_parts(&self, q: &Components, c: &Components, mode: Mode) -> (Verdict, Vec<&'static str>) {
        // "1-2-3405" vs "1-2-3" + room 405: a room glued to the block number (CJK listings)
        if let (Some(a), Some(b)) = (&q.house, &c.house) {
            if a != b && a.contains('-') && b.contains('-') {
                let fix = |long: &Components, short: &Components| -> Option<Components> {
                    let (lh, sh) = (long.house.as_ref()?, short.house.as_ref()?);
                    let rest = lh.strip_prefix(sh.as_str())?;
                    if rest.is_empty() || !rest.chars().all(|ch| ch.is_ascii_digit()) || !short.units.iter().any(|u| &u[1..] == rest) {
                        return None;
                    }
                    let mut l = long.clone();
                    l.house = Some(sh.clone());
                    l.units.push(format!("u{}", rest));
                    Some(l)
                };
                if let Some(q2) = fix(q, c) {
                    return self.compare_parsed(&q2, c, mode);
                }
                if let Some(c2) = fix(c, q) {
                    return self.compare_parsed(q, &c2, mode);
                }
            }
        }
        let typo = matches!(mode, Mode::Query);
        let peer = matches!(mode, Mode::Peer);
        let mut reasons = Vec::new();
        let mut unknown = false;
        // PO box
        match (&q.po_box, &c.po_box) {
            (Some(a), Some(b)) if a != b => return (Verdict::Different, vec!["po_box"]),
            (Some(_), None) | (None, Some(_)) if !peer => return (Verdict::Different, vec!["po_box_missing"]),
            _ => {}
        }
        // a second number of the house number's part, when both sides give one
        if let (Some(a), Some(b)) = (&q.second_number, &c.second_number) {
            if a != b {
                return (Verdict::Different, vec!["house_number"]);
            }
        }
        // an Irish postal district, when both sides give one ("Dublin 2" is not "Dublin 4")
        if let (Some(a), Some(b)) = (&q.postal_district, &c.postal_district) {
            if a != b {
                return (Verdict::Different, vec!["city"]);
            }
        }
        // house number
        match (&q.house, &c.house) {
            (Some(a), Some(b)) => {
                if a != b {
                    // "73 K Zámku" read as house "73k": the letter may start the street ("K Zámku 73")
                    if let Some(x2) = Self::letter_to_street(q, c).or_else(|| Self::letter_to_street(c, q)) {
                        let (qq, cc2) = if q.house.as_ref().map_or(0, |h| h.len()) > c.house.as_ref().map_or(0, |h| h.len()) { (&x2, c) } else { (q, &x2) };
                        return self.compare_parsed(qq, cc2, mode);
                    }
                    return (Verdict::Different, vec!["house_number"]);
                }
            }
            (Some(_), None) => {
                if q.po_box.is_none() {
                    if peer {
                        return (Verdict::Different, vec!["house_number_missing"]);
                    }
                    unknown = true;
                    reasons.push("candidate_has_no_house_number");
                }
            }
            (None, Some(_)) => {
                if q.po_box.is_none() {
                    return (Verdict::Different, vec!["candidate_adds_house_number"]);
                }
            }
            (None, None) => {}
        }
        // a compass word on one street and not the other, or another one, is another street, even
        // where it reads as a state code too ("1 Example St NE" is not "1 Example St" in Nebraska)
        if !q.street.is_empty() && !c.street.is_empty() && is_latinish(q.street.script) && is_latinish(c.street.script) && street_directions(&q.street).0 != street_directions(&c.street).0 && street_directions(&q.street).1 == street_directions(&c.street).1 {
            return (Verdict::Different, vec!["street_name"]);
        }
        // street name
        let compound = self.compound_eq(q, c) || self.compound_eq(c, q) || self.full_street_eq(q, c, typo);
        if compound {
            // "knutsengata" vs "knutsen gt.": same street, the type is inside the compound
        } else if !q.street.is_empty() && !c.street.is_empty() {
            match text_eq(self, &q.street, &c.street, typo) {
                Tri::Yes => {}
                Tri::No if typo && self.compound_typo(q, c) => {
                    // "Keelopolku" vs "Kielopolku": one typo in the whole word, the type inside it
                }
                Tri::No => {
                    // an address written without commas keeps the town inside the street part:
                    // let the other side's street show where the street ends
                    let (rq, rc) = (self.realign(q, c, typo), self.realign(c, q, typo));
                    if let Ok(q2) = rq {
                        return self.compare_parsed(&q2, c, mode);
                    }
                    if let Ok(c2) = rc {
                        return self.compare_parsed(q, &c2, mode);
                    }
                    // the street matched once the town was split off, but the town did not
                    if matches!(rq, Err(true)) || matches!(rc, Err(true)) {
                        return (Verdict::Different, vec!["city"]);
                    }
                    return (Verdict::Different, vec!["street_name"]);
                }
                Tri::Unknown => {
                    unknown = true;
                    reasons.push("street_name_not_comparable");
                }
            }
        } else if q.street.is_empty() != c.street.is_empty() && q.po_box.is_none() {
            unknown = true;
            reasons.push("street_missing");
        }
        // street type
        if !compound && !q.street_type.is_empty() && !c.street_type.is_empty() && !intersects(&q.street_type, &c.street_type) {
            let (qw, cw) = (&q.street_type_word, &c.street_type_word);
            // a guessed abbreviation may stand for any type it abbreviates; a dictionary abbreviation
            // only for its own expansions, unless it is a dotted prefix of 3+ letters ("vic." vicolo)
            let ab = |x: &Components, xw: &str, yw: &str| (x.street_type_guess && parse::is_abbrev(xw, yw)) || (x.street_type_abbr && xw.len() >= 3 && yw.starts_with(xw));
            // Nordic definite forms: "väg" / "vägen", "gata" / "gatan" ("v.", "g." abbreviate both)
            let definite = |x: &Components, yw: &str| {
                x.street_type.iter().any(|cid| {
                    let ck = &self.data.canon_keys[*cid as usize];
                    NORDIC_TYPES.contains(&ck.as_str()) && yw.len() > ck.len() && yw.starts_with(ck.as_str()) && matches!(&yw[ck.len()..], "en" | "n" | "et" | "t" | "an" | "a" | "e" | "na" | "ne")
                })
            };
            // two abbreviations of one word ("ch." and "chem." for chemin)
            let both_abbr = q.street_type_abbr && c.street_type_abbr && (parse::is_abbrev(qw, cw) || parse::is_abbrev(cw, qw));
            let abbr_ok = ab(q, qw, cw) || ab(c, cw, qw) || definite(q, cw) || definite(c, qw) || both_abbr;
            let cross = is_latinish(q.street_type_script) != is_latinish(c.street_type_script);
            if abbr_ok {
            } else if cross {
                if qw == cw || (!qw.is_empty() && fold::skeleton(qw) == fold::skeleton(cw)) || same_type_group(qw, cw) {
                } else {
                    unknown = true;
                    reasons.push("street_type_not_comparable");
                }
            } else {
                return (Verdict::Different, vec!["street_type"]);
            }
        }
        // units
        if !c.units.is_empty() {
            if q.units.is_empty() {
                if !peer {
                    return (Verdict::Different, vec!["candidate_adds_unit"]);
                }
            } else {
                let qn: Vec<String> = q.units.iter().map(|u| u[1..].to_string()).collect();
                let cn: Vec<String> = c.units.iter().map(|u| u[1..].to_string()).collect();
                // every unit value the candidate gives must be one the caller gave
                if !cn.iter().all(|u| qn.contains(u)) || (!peer && !qn.iter().all(|u| cn.contains(u)) && cn.len() >= qn.len()) {
                    return (Verdict::Different, vec!["unit"]);
                }
            }
        } else if peer && !q.units.is_empty() {
            // fine: unit missing on one listing
        }
        // postcode
        if let (Some(a), Some(b)) = (&q.postcode, &c.postcode) {
            if a != b {
                return (Verdict::Different, vec!["postcode"]);
            }
        }
        // another state, province or country ("IL" and "Missouri", "Jiangsu" and "福建省")
        if self.regions_differ(q, c) || self.town_region_differs(q, c) || self.town_region_differs(c, q) {
            return (Verdict::Different, vec!["region"]);
        }
        if self.countries_differ(q, c) || self.crosses_hint(c) != self.crosses_hint(q) {
            return (Verdict::Different, vec!["country"]);
        }
        // two listings that place themselves by different means (one by town, one by postcode only)
        // cannot be shown to be one place
        if peer {
            let only_town = |x: &Components| x.city.is_some() && x.postcode.is_none();
            let only_code = |x: &Components| x.city.is_none() && x.postcode.is_some();
            if (only_town(q) && only_code(c)) || (only_code(q) && only_town(c)) {
                return (Verdict::Different, vec!["no_shared_location"]);
            }
        }
        // city / locality
        if let (Some(qc), Some(cc)) = (&q.city, &c.city) {
            // two listings name their own towns: different towns are different places
            // a later place name that is a region ("San Antonio, Valparaíso") is not a town, even when
            // a town shares its name
            // (CJK addresses run from the region down, so the order says nothing there)
            let region_role = |x: &Components, i: usize| i > 0 && x.regions.contains(&x.localities[i].key) && !matches!(x.script, Some(Script::Han) | Some(Script::Kana) | Some(Script::Hangul));
            let named_in = |t: &Text, x: &Components| x.localities.iter().enumerate().any(|(i, l)| !region_role(x, i) && (text_eq(self, t, l, false) == Tri::Yes || within(&t.key, &l.key) || within(&l.key, &t.key)));
            if peer && !named_in(qc, c) && !named_in(cc, q) {
                return (Verdict::Different, vec!["city"]);
            }
            let mut any_yes = false;
            let mut any_unknown = false;
            // a town that matches only within a typo ("Exampletn" for "Exampleton"), where towns
            // share names across states (a romanized town keeps its allowance: "Backa" for "Bačka")
            let mut misspelled = false;
            let spelling = q.cc.as_deref().map_or(false, |cc| POSTAL_CODE_COUNTRIES.contains(&cc));
            // a town matching only the other side's region counts when that side named no other town;
            // when the town side gave just that one name ("Valparaíso" vs "San Antonio, Valparaíso") it
            // may be the town or the region: not settled
            // 0: an ordinary match, 1: ignore, 2: not settled
            // an equal postcode on both sides settles it
            let same_code = q.postcode.is_some() && q.postcode == c.postcode;
            let region_kind = |owner: &Components, town_side: &Components, t: &Text, role: bool| -> u8 {
                if !role || same_code || owner.city.as_ref().map_or(true, |oc| owner.regions.contains(&oc.key)) {
                    0
                } else if town_side.localities.len() == 1 && town_side.regions.contains(&t.key) {
                    2
                } else {
                    1
                }
            };
            let pairs = c
                .localities
                .iter()
                .enumerate()
                .map(|(i, b)| (qc, b, region_kind(c, q, qc, region_role(c, i))))
                .chain(q.localities.iter().enumerate().map(|(i, a)| (a, cc, region_kind(q, c, cc, region_role(q, i)))));
            for (a, b, region) in pairs {
                // one edit in a town name is only a typo for long names ("Lublin" and "Lubin" are two towns)
                let typo_town = typo && letters(&a.key).min(letters(&b.key)) >= 7;
                match text_eq(self, a, b, typo_town) {
                    Tri::Yes if region == 1 => {}
                    Tri::Yes if region == 2 => any_unknown = true,
                    Tri::Yes if spelling && typo_town && text_eq(self, a, b, false) != Tri::Yes => misspelled = true,
                    Tri::Yes => any_yes = true,
                    Tri::Unknown => any_unknown = true,
                    Tri::No => {}
                }
            }
            // a caller business segment may actually be a locality in candidate order
            // "Lubin dolnośląskie" written without commas: the town run may end (or start) with a
            // place name the candidate lists separately, or with a country name
            if !any_yes && !peer && qc.toks.len() >= 2 {
                let known = |k: &str| self.data.is_country_name(k) || c.localities.iter().any(|l| l.key == k) || c.regions.iter().any(|r| r == k) || self.is_region(q, k);
                let n = qc.toks.len();
                for cut in 1..n {
                    for (keep, drop) in [(0..cut, cut..n), (cut..n, 0..cut)] {
                        let dk: String = qc.toks[drop].iter().map(|t| t.0.as_str()).collect();
                        if !known(&dk) {
                            continue;
                        }
                        let toks = qc.toks[keep.clone()].to_vec();
                        let key: String = toks.iter().map(|t| t.0.as_str()).collect();
                        let t = Text { skel: fold::skeleton(&key), key, script: qc.script, abbr: qc.abbr.get(keep.clone()).map_or(vec![false; toks.len()], |a| a.to_vec()), toks, native: String::new() };
                        if c.localities.iter().any(|l| text_eq(self, &t, l, false) == Tri::Yes) {
                            any_yes = true;
                        }
                    }
                }
            }
            // with `Rules::district_may_differ`, outside the US, a town written with its district
            // after it ("München-Altstadt-Lehel") is the caller's town when its first word is that
            // town and both give one postcode
            let mut district_town = false;
            // a Danish town written with its postal district's letters ("København V") or as its
            // municipality ("Copenhagen Municipality") beside the town written another of those ways;
            // two districts must still agree ("Odense S" is not "Odense NØ")
            if !any_yes && !peer && q.cc.as_deref() == Some("DK") {
                let last = |x: &Text| if x.toks.len() >= 2 { x.toks[x.toks.len() - 1].0.clone() } else { String::new() };
                // only the towns whose postal districts carry letters ("Nykøbing M" is a town of its own)
                let stem = |x: &Text| -> String {
                    let joined: String = x.toks[..x.toks.len().saturating_sub(1)].iter().map(|t| t.0.as_str()).collect();
                    self.data.exonyms.get(&joined).cloned().unwrap_or(joined)
                };
                let district = |x: &Text| DANISH_DISTRICTS.contains(&last(x).as_str()) && DANISH_DISTRICT_TOWNS.contains(&stem(x).as_str());
                let tail = |x: &Text| district(x) || matches!(last(x).as_str(), "municipality" | "kommune");
                let base = |x: &Text| -> Text {
                    let n = if tail(x) { x.toks.len() - 1 } else { x.toks.len() };
                    let joined: String = x.toks[..n].iter().map(|t| t.0.as_str()).collect();
                    let key = self.data.exonyms.get(&joined).cloned().unwrap_or(joined);
                    Text { skel: fold::skeleton(&key), toks: vec![(key.clone(), Vec::new())], key, script: x.script, abbr: vec![false], native: String::new() }
                };
                if (tail(qc) || tail(cc)) && !(district(qc) && district(cc)) && text_eq(self, &base(qc), &base(cc), false) == Tri::Yes {
                    any_yes = true;
                    district_town = true;
                }
            }
            if !any_yes && !peer && same_code && q.cc.as_deref().map_or(false, |cc| !US_POSTAL.contains(&cc)) && DISTRICT_MAY_DIFFER.with(|cell| cell.get()) {
                let head = |x: &Text| -> Option<Text> {
                    if x.toks.len() < 2 {
                        return None;
                    }
                    let first = x.toks[0].0.clone();
                    let key = self.data.exonyms.get(&first).cloned().unwrap_or(first);
                    Some(Text { skel: fold::skeleton(&key), toks: vec![(key.clone(), Vec::new())], key, script: x.script, abbr: vec![false], native: String::new() })
                };
                let town_first = |town: &Text, written: &Text| head(written).map_or(false, |h| text_eq(self, town, &h, false) == Tri::Yes);
                if town_first(qc, cc) || town_first(cc, qc) {
                    any_yes = true;
                    district_town = true;
                }
            }
            if !any_yes {
                // the caller's town spelled another way may be the row's or another town: ask
                if misspelled && !any_unknown {
                    unknown = true;
                    reasons.push("town_spelled_differently");
                } else if any_unknown {
                    unknown = true;
                    reasons.push("city_not_comparable");
                } else {
                    return (Verdict::Different, vec!["city"]);
                }
            } else if !peer && !district_town {
                // the right area: a place name on each side that the other side does not have
                // ("Santa Sofia" vs "Dovadola") is not settled by one shared word
                // region names count too ("Port Louis" is a district and a town), but two different
                // region levels ("Bergamo" province, "Lombardia" region) do not conflict
                let towns = |x: &Components| -> Vec<(Text, bool)> { x.localities.iter().filter(|t| !self.data.is_country_name(&t.key)).map(|t| (t.clone(), x.regions.contains(&t.key))).collect() };
                let (qt, ct) = (towns(q), towns(c));
                let unmatched = |a: &[(Text, bool)], b: &[(Text, bool)]| -> Vec<bool> {
                    a.iter().filter(|(t, _)| !b.iter().any(|(u, _)| text_eq(self, t, u, typo) != Tri::No || within(&u.key, &t.key) || within(&t.key, &u.key))).map(|(_, r)| *r).collect()
                };
                let (uq, uc) = (unmatched(&qt, &ct), unmatched(&ct, &qt));
                if uq.iter().any(|&rq| uc.iter().any(|&rc| !(rq && rc))) {
                    unknown = true;
                    reasons.push("locality_unmatched");
                }
            }
        }
        if unknown {
            return (Verdict::Unknown, reasons);
        }
        (Verdict::Same, reasons)
    }

    /// Words of a name, folded, each with its dictionary meanings ("St" and "Street" share one).
    fn name_words(&self, text: &str) -> Vec<(String, Vec<u32>)> {
        fold::fold(text)
            .split(|ch: char| !ch.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(|w| {
                let mut cs: Vec<u32> = self.data.lookup(w).iter().filter(|e| matches!(e.class, data::Class::StreetType | data::Class::Synonym | data::Class::Building | data::Class::Directional | data::Class::Title)).map(|e| e.canon).collect();
                cs.sort();
                cs.dedup();
                (fold::ascii_key(w), cs)
            })
            .filter(|w| !w.0.is_empty())
            .collect()
    }

    /// The caller named a place rather than a street address: no house number or PO box, and the
    /// first part is words (an ordinal such as "5th" may start it). Returns the name's words.
    fn place_name(&self, query: &str, q: &Components) -> Option<Vec<(String, Vec<u32>)>> {
        if q.po_box.is_some() || !q.units.is_empty() {
            return None;
        }
        let first = query.split(',').next().unwrap_or("");
        let words = self.name_words(&self.without_short_codes(first));
        if words.is_empty() || !words.iter().any(|w| w.0.chars().any(|ch| ch.is_ascii_alphabetic())) {
            return None;
        }
        // digits in the first part are a house number, unless an ordinal leads the name ("5th")
        let ordinal = |w: &str| {
            let d = w.trim_end_matches(|ch: char| ch.is_ascii_alphabetic());
            !d.is_empty() && d.chars().all(|ch| ch.is_ascii_digit()) && matches!(&w[d.len()..], "st" | "nd" | "rd" | "th")
        };
        if words.iter().enumerate().any(|(i, w)| w.0.chars().any(|ch| ch.is_ascii_digit()) && !(i == 0 && ordinal(&w.0))) {
            return None;
        }
        match &q.house {
            None => Some(words),
            // "5th Street Market": the ordinal was read as a house number
            Some(h) if words[0].0.starts_with(h.as_str()) && words[0].0.len() > h.len() && !query.split(',').skip(1).any(|p| p.chars().any(|ch| ch.is_ascii_digit()) && p.split_whitespace().any(|w| w.chars().all(|ch| ch.is_ascii_digit()) && w.len() < 4)) => Some(words),
            _ => None,
        }
    }

    /// The short codes written in parentheses in the name of a transport place, such as an
    /// airport's or a station's letters ("Example International Airport (EXA)"): three capital
    /// letters, after a name that holds a transport word (`TRANSPORT_WORDS`). A Roman numeral
    /// ("(III)"), a compass point ("(NNE)") or a direction the dictionaries know is not one, and nor
    /// is any code beside another kind of name ("Example Bank (ATM)", "Example Plaza (II)",
    /// "Example Park (IL)"). Folded.
    fn short_codes(&self, part: &str) -> Vec<String> {
        let mut groups = Vec::new();
        let mut outside = String::new();
        let mut rest = part;
        while let Some(open) = rest.find('(') {
            let after = &rest[open + 1..];
            let Some(close) = after.find(')') else { break };
            outside.push_str(&rest[..open]);
            outside.push(' ');
            groups.push(after[..close].trim());
            rest = &after[close + 1..];
        }
        outside.push_str(rest);
        if groups.is_empty() || !self.name_words(&outside).iter().any(|w| TRANSPORT_WORDS.contains(&w.0.as_str())) {
            return Vec::new();
        }
        groups
            .into_iter()
            .filter(|g| g.len() == 3 && g.chars().all(|ch| ch.is_ascii_uppercase()))
            .map(|g| g.to_ascii_lowercase())
            .filter(|k| !k.chars().all(|ch| matches!(ch, 'i' | 'v' | 'x')) && !COMPASS_POINTS.contains(&k.as_str()) && !self.data.lookup(k).iter().any(|e| e.class == data::Class::Directional))
            .collect()
    }

    /// A place's name without its short codes in parentheses (`short_codes`), so that "Example
    /// International Airport (EXA)" is compared as "Example International Airport". Any other
    /// parenthesis ("(Gate B)", "(B gates)") stays: it may name a part of the place.
    fn without_short_codes<'t>(&self, part: &'t str) -> std::borrow::Cow<'t, str> {
        let codes = self.short_codes(part);
        if codes.is_empty() {
            return std::borrow::Cow::Borrowed(part);
        }
        let mut out = String::with_capacity(part.len());
        let mut rest = part;
        while let Some(open) = rest.find('(') {
            let after = &rest[open + 1..];
            let Some(close) = after.find(')') else { break };
            let inner = after[..close].trim();
            out.push_str(&rest[..open]);
            if !codes.contains(&inner.to_ascii_lowercase()) {
                out.push_str(&rest[open..open + close + 2]);
            }
            rest = &after[close + 1..];
        }
        out.push_str(rest);
        std::borrow::Cow::Owned(out)
    }

    /// A row as a caller's place name is compared with it, and whether it names a part of the
    /// caller's place. A dash with a space on each side in the row's first part separates the name
    /// from what follows, as a comma does, only when what follows (up to the next comma) is an
    /// address: a house number and street ("Example Park — 12 Sample Plaza"), a postcode, or a town
    /// the caller gave ("Example Park — Exampleton, CA"). When what follows is the caller's own name
    /// and what comes before names a part by a number or a place-type word ("Terminal 2 — Example
    /// International Airport"), the row is a part of the caller's place.
    /// Otherwise the dash is part of the name: "Example Stadium - Gate A" and "Example Inn -
    /// Airport" are other places, and a caller who types "Example - Sample Stadium" is that name.
    fn named_row<'t>(&self, ctx: &Ctx, text: &'t str, name: &[(String, Vec<u32>)], qa: &Components) -> (std::borrow::Cow<'t, str>, bool) {
        let first_end = text.find(',').unwrap_or(text.len());
        let chars: Vec<(usize, char)> = text[..first_end].char_indices().collect();
        let dash = (1..chars.len().saturating_sub(1)).find(|&i| matches!(chars[i].1, '-' | '\u{2013}' | '\u{2014}') && chars[i - 1].1.is_whitespace() && chars[i + 1].1.is_whitespace());
        let Some(d) = dash else { return (std::borrow::Cow::Borrowed(text), false) };
        let (at, len) = (chars[d].0, chars[d].1.len_utf8());
        let head = text[..at].trim();
        let segment = text[at + len..first_end].trim();
        if head.is_empty() || segment.is_empty() {
            return (std::borrow::Cow::Borrowed(text), false);
        }
        // a part of the place before its name: a number or a place-type word the caller did not write
        // ("Terminal 2", "Food Court"), not a brand ("Example - Sample Stadium")
        let part = self.name_words(head).iter().any(|w| w.0.chars().any(|ch| ch.is_ascii_digit()) || (!name.iter().any(|n| n.0 == w.0) && self.data.lookup(&w.0).iter().any(|e| e.class == data::Class::Place)));
        if part && self.name_relation(name, &self.name_words(&self.without_short_codes(segment))) == 2 {
            return (std::borrow::Cow::Borrowed(text), true);
        }
        let street = parse::parse(ctx, segment);
        // a house number with a street type or a building word ("12 Sample Plaza", "1 Example Bldg"),
        // not a count or a numbered part ("24 Hour Parking", "12 Screens")
        let address = street.house.is_some() && !street.street.is_empty() && (!street.street_type.is_empty() || segment.split_whitespace().any(|w| self.data.lookup(&w.to_lowercase()).iter().any(|e| e.class == data::Class::Building)));
        let area = Self::area_parse(ctx, segment);
        let postcode = area.postcode.is_some() && area.house.is_none() && area.street.is_empty() && !segment.chars().any(|ch| ch.is_alphabetic() && !ch.is_ascii_uppercase());
        let callers_town = self.towns(&area).iter().any(|u| self.towns(qa).iter().any(|t| text_eq(self, t, u, false) == Tri::Yes));
        if address || postcode || callers_town {
            (std::borrow::Cow::Owned(format!("{}, {}", head, &text[at + len..].trim_start())), false)
        } else {
            (std::borrow::Cow::Borrowed(text), false)
        }
    }

    /// A text of a name with no digits and then a lone number of four or more digits, after a
    /// comma or not: the name, a comma and the number read as a postcode ("1 Example St, Exampleton"
    /// is no such text).
    fn name_and_postcode(query: &str) -> Option<String> {
        let parts: Vec<&str> = query.split(',').map(|p| p.trim()).filter(|p| !p.is_empty()).collect();
        let (name, code) = match parts.as_slice() {
            [name, code] => (name.to_string(), code.to_string()),
            [one] => {
                let (name, code) = one.rsplit_once(' ')?;
                (name.trim().to_string(), code.to_string())
            }
            _ => return None,
        };
        let postcode = code.len() >= 4 && code.chars().all(|ch| ch.is_ascii_digit());
        let named = name.chars().any(|ch| ch.is_alphabetic()) && !name.chars().any(|ch| ch.is_ascii_digit());
        (postcode && named).then(|| format!("{}, {}", name, code))
    }

    /// How a row's first part relates to the caller's place name: 2 the same name, 1 a variant
    /// (the caller's words in order with words added before or between them: "The Example Park",
    /// "Jane Q. Example Convention Center" for "Example Center"), 0 another place (another name, or
    /// the name followed by more words: "Example Park Ferry Terminal").
    fn name_relation(&self, name: &[(String, Vec<u32>)], row: &[(String, Vec<u32>)]) -> u8 {
        let eq = |a: &(String, Vec<u32>), b: &(String, Vec<u32>)| a.0 == b.0 || intersects(&a.1, &b.1);
        // a number the caller did not write makes the row an address, not the place's name
        if row.iter().any(|w| w.0.chars().all(|ch| ch.is_ascii_digit()) && !name.iter().any(|n| n.0 == w.0)) {
            return 0;
        }
        if row.len() == name.len() && name.iter().zip(row).all(|(a, b)| eq(a, b)) {
            return 2;
        }
        if row.len() <= name.len() {
            return 0;
        }
        if name.iter().zip(row).all(|(a, b)| eq(a, b)) {
            return 0;
        }
        let mut j = 0;
        for w in row {
            if j < name.len() && eq(&name[j], w) {
                j += 1;
            }
        }
        if j == name.len() {
            1
        } else {
            0
        }
    }

    /// A longer name a read must not take for the caller's place: it does not end with the caller's
    /// name (words follow the caller's last word: "The Example Park Hotel", "Jane Q. Example Park
    /// Suites"), or a word it adds is a compass, age or size word ("North Example Park", "Old Example
    /// Park"). Given names and initials before the name ("Jane Q. Example Convention Center" for
    /// "Example Center") are fine.
    fn named_after(&self, name: &[(String, Vec<u32>)], row: &[(String, Vec<u32>)]) -> bool {
        let eq = |a: &(String, Vec<u32>), b: &(String, Vec<u32>)| a.0 == b.0 || intersects(&a.1, &b.1);
        let (Some(last), Some(row_last)) = (name.last(), row.last()) else {
            return true;
        };
        if !eq(last, row_last) {
            return true;
        }
        // the caller's other words, in order, before the row's last word; every other row word is added
        let mut matched = vec![false; row.len()];
        matched[row.len() - 1] = true;
        let mut j = 0;
        for (i, w) in row[..row.len() - 1].iter().enumerate() {
            if j + 1 < name.len() && eq(&name[j], w) {
                matched[i] = true;
                j += 1;
            }
        }
        if j + 1 != name.len() {
            return true;
        }
        row.iter().zip(&matched).any(|(w, &m)| !m && (AGE_WORDS.contains(&w.0.as_str()) || self.data.lookup(&w.0).iter().any(|e| e.class == data::Class::Directional)))
    }

    /// A site's control rather than a place: the row holds the caller's whole text, and every other
    /// word in it is a control word ("see", "search", "more", "results", "use" ...), a filler ("for",
    /// "all") or a count, with at least one control word: "See more results for <text>",
    /// "Search "<text>"", "<text> (see more)". A row that adds a name ("Jane Doe <text>", "Old Town
    /// <text>", "Exampleton Central Station") is a place, not a control.
    pub fn is_control(&self, query: &str, row: &str) -> bool {
        let q: Vec<String> = self.name_words(query).into_iter().map(|w| w.0).collect();
        let r: Vec<String> = self.name_words(row).into_iter().map(|w| w.0).collect();
        if q.is_empty() || r.len() <= q.len() {
            return false;
        }
        (0..=r.len() - q.len()).any(|at| {
            if r[at..at + q.len()] != q[..] {
                return false;
            }
            let mut control = false;
            for w in r[..at].iter().chain(&r[at + q.len()..]) {
                if CONTROL_WORDS.contains(&w.as_str()) {
                    control = true;
                } else if !CONTROL_FILLERS.contains(&w.as_str()) && !w.chars().all(|ch| ch.is_ascii_digit()) {
                    return false;
                }
            }
            control
        })
    }

    /// The row shows the caller's whole text, of two parts or more, starting inside its own first
    /// part after at least `min_words` words of it ("Search "<text>"", "The <text>").
    pub fn shows_typed_text(&self, query: &str, row: &str, min_words: usize) -> bool {
        if !query.contains(',') {
            return false;
        }
        let q: Vec<String> = self.name_words(query).into_iter().map(|w| w.0).collect();
        let r: Vec<String> = self.name_words(row).into_iter().map(|w| w.0).collect();
        let first_len = self.name_words(row.split(',').next().unwrap_or("")).len();
        if q.is_empty() || r.len() < q.len() + min_words.max(1) || r[0].chars().any(|ch| ch.is_ascii_digit()) {
            return false;
        }
        (min_words.max(1)..=r.len() - q.len()).any(|at| {
            at < first_len && r[at..at + q.len()] == q[..] && (at + q.len() == r.len() || (at + q.len() + 1 == r.len() && r[r.len() - 1].chars().all(|ch| ch.is_ascii_digit())) || min_words < 2)
        })
    }

    /// The caller's house number, street and town, with only the postcode different: the same
    /// building under a neighbouring postcode, or a postcode typed wrong. A picker reading a site may
    /// accept it; a write asks. Both sides must name the same town, the same districts where either
    /// gives one, and no other region; the postcodes may differ only in their last two characters
    /// (one postal area); and in a city that repeats street names across its districts (Berlin,
    /// Hamburg, London, Vienna, Budapest, Prague) both must name the same district.
    pub fn same_but_postcode(&self, query: &str, row: &str, country: Option<&str>) -> bool {
        let ((query, ql), (row, rl)) = (self.bound(query), self.bound(row));
        if ql || rl {
            return false;
        }
        let (query, row) = (&*query, &*row);
        let ctx = self.ctx(country);
        self.postcode_only_differs(&parse::parse(&ctx, query), &parse::parse(&ctx, row))
    }

    /// `same_but_postcode` on parsed text.
    fn postcode_only_differs(&self, q: &Components, c: &Components) -> bool {
        let (Some(a), Some(b)) = (&q.postcode, &c.postcode) else {
            return false;
        };
        if q.house.is_none() || a == b {
            return false;
        }
        let key = |p: &str| -> Vec<char> { p.chars().filter(|ch| ch.is_alphanumeric()).flat_map(|ch| ch.to_lowercase()).collect() };
        let (a, b) = (key(a), key(b));
        // a United Kingdom postcode's outward code is its district ("W1D" of "W1D 1AN"); one street
        // name is not repeated within a district
        let outward = |p: &[char]| -> Option<String> {
            let s: String = p.iter().collect();
            static GB: std::sync::OnceLock<regex_lite::Regex> = std::sync::OnceLock::new();
            let gb = GB.get_or_init(|| regex_lite::Regex::new(r"^[a-z]{1,2}\d[a-z\d]?\d[a-z]{2}$").unwrap());
            (p.len() >= 5 && gb.is_match(&s)).then(|| s[..s.len() - 3].to_string())
        };
        let one_district = matches!((outward(&a), outward(&b)), (Some(x), Some(y)) if x == y);
        if !one_district && (a.len() != b.len() || a.len() < 3 || a[..a.len() - 2] != b[..b.len() - 2]) {
            return false;
        }
        let (Some(qc), Some(cc)) = (&q.city, &c.city) else {
            return false;
        };
        if text_eq(self, qc, cc, false) != Tri::Yes {
            return false;
        }
        let (qt, ct) = (self.towns(q), self.towns(c));
        let all_named = |x: &[Text], y: &[Text]| x.iter().all(|t| y.iter().any(|u| text_eq(self, t, u, false) == Tri::Yes));
        if !all_named(&qt, &ct) || !all_named(&ct, &qt) {
            return false;
        }
        if REPEATED_STREET_CITIES.contains(&qc.key.as_str()) && qt.len() < 2 && !one_district {
            return false;
        }
        let (mut q, mut c) = (q.clone(), c.clone());
        q.postcode = None;
        c.postcode = None;
        self.compare_parsed(&q, &c, Mode::Query).0 == Verdict::Same
    }

    /// The text after a place's name: a street, if any, then the area. Read as an address it may
    /// lose its town ("Exampleton, CA 90000" reads as house 90000 on "CA" street); then the area is
    /// read as if a street came first.
    fn area_parse(ctx: &Ctx, text: &str) -> Components {
        let p = parse::parse(ctx, text);
        if p.city.is_some() || !p.localities.is_empty() || !p.street_type.is_empty() || text.trim().is_empty() {
            return p;
        }
        let mut a = parse::parse(ctx, &format!("1 Example St, {}", text));
        a.house = None;
        a.street = Text::default();
        a.street_type.clear();
        a
    }

    /// A text's place names that are neither a region nor a country, and its town when the town is
    /// named after a region and another region follows it ("Washington" in "Washington, PA").
    fn towns(&self, x: &Components) -> Vec<Text> {
        let city_is_town = !self.region_ids(x, false).is_empty();
        x.localities
            .iter()
            .filter(|t| !x.countries.contains(&t.key) && (!x.regions.contains(&t.key) || (city_is_town && t.key.len() > 2 && x.city.as_ref().map_or(false, |c| c.key == t.key))))
            .cloned()
            .collect()
    }

    /// The regions a place name may name in a country: by its characters when it is written in Han
    /// (山西省 and 陕西省 read alike), else by its key, where a shared key names each region it may.
    fn name_regions(co: &data::Country, t: &Text) -> Vec<u16> {
        // the region lists hold Han names in Han, with and without their kind suffix; "鹿児島市" (a
        // city) is not "鹿児島県"
        if let Some(h) = han_name(t) {
            return co.region_ids(&h);
        }
        co.region_ids(&t.key)
    }

    /// The regions a text names (indexes in its country's `region_info`), each with the lists that
    /// name it and whether it is a city: its regions and codes, and place names that are a region's
    /// short name ("Jiangsu"). Its town counts only when no other part names a region, and then when
    /// the region is a city ("Taichung" for 台中市) or with `with_town`; a town before another region
    /// is a town named after a region ("Washington, PA", "Nevada, MO").
    fn region_ids(&self, x: &Components, with_town: bool) -> Vec<(u16, u8, bool)> {
        let Some(co) = x.cc.as_deref().and_then(|c| self.data.countries.get(c)) else {
            return Vec::new();
        };
        let city = x.city.as_ref().map(|c| c.key.as_str());
        let mut parts: Vec<(bool, Vec<u16>)> = Vec::new();
        // the town is the first place name with its key ("Washington, Washington": the town, then
        // the state)
        let mut town_seen = false;
        for t in x.localities.iter().filter(|t| !x.countries.contains(&t.key)) {
            // a two-letter code is a state's code, never a town ("Example St Exampleton, NE")
            let is_town = !town_seen && Some(t.key.as_str()) == city && t.key.len() > 2;
            town_seen |= is_town;
            let ids = Self::name_regions(co, t);
            if !ids.is_empty() {
                parts.push((is_town, ids));
            }
        }
        for k in x.regions.iter().filter(|k| !x.localities.iter().any(|t| &t.key == *k)) {
            let ids = co.region_ids(k);
            if !ids.is_empty() {
                parts.push((false, ids));
            }
        }
        let others = parts.iter().any(|p| !p.0);
        // a town alone is a region when the region is a city ("Taichung", "New Taipei"): a state's
        // name as a town ("Washington") may be either
        let city_region = |ids: &[u16]| ids.iter().all(|&id| co.region_info.get(id as usize).map_or(false, |r| r.city));
        let mut v: Vec<(u16, u8, bool)> = parts
            .into_iter()
            .filter(|p| !p.0 || (!others && (with_town || city_region(&p.1))))
            .flat_map(|p| p.1)
            .map(|id| {
                let info = co.region_info.get(id as usize).copied().unwrap_or_default();
                (id, info.sources, info.city)
            })
            .collect();
        v.sort();
        v.dedup();
        v
    }

    /// The two texts name different regions of one level ("IL" and "Missouri", "Jiangsu" and
    /// "福建省"). A town named after a region is not that region.
    fn regions_differ(&self, a: &Components, b: &Components) -> bool {
        if self.zip_outside_state(a, b) || self.zip_outside_state(b, a) {
            return true;
        }
        let (ra, rb) = (self.region_ids(a, false), self.region_ids(b, false));
        [1u8, 2].iter().any(|&bit| {
            let xa: Vec<u16> = ra.iter().filter(|r| r.1 & bit != 0).map(|r| r.0).collect();
            let xb: Vec<u16> = rb.iter().filter(|r| r.1 & bit != 0).map(|r| r.0).collect();
            !xa.is_empty() && !xb.is_empty() && !xa.iter().any(|i| xb.contains(i))
        })
    }

    /// The states a United States ZIP code may be in, by its first three digits.
    fn zip_states(&self, x: &Components) -> Vec<u16> {
        let (Some("US"), Some(p)) = (x.cc.as_deref(), x.postcode.as_deref()) else {
            return Vec::new();
        };
        let Some(co) = self.data.countries.get("US") else {
            return Vec::new();
        };
        let d = p.as_bytes();
        if d.len() < 5 || !d[..5].iter().all(|b| b.is_ascii_digit()) {
            return Vec::new();
        }
        let prefix = (d[0] - b'0') as u16 * 100 + (d[1] - b'0') as u16 * 10 + (d[2] - b'0') as u16;
        US_ZIP_PREFIXES.iter().filter(|r| r.0 <= prefix && prefix <= r.1).flat_map(|r| r.2.split(' ')).flat_map(|k| co.region_ids(k)).collect()
    }

    /// One text gives a state and the other a ZIP code of another state ("..., KS" and "... 64100").
    fn zip_outside_state(&self, a: &Components, b: &Components) -> bool {
        let zone = self.zip_states(b);
        let states: Vec<u16> = self.region_ids(a, false).iter().map(|r| r.0).collect();
        !zone.is_empty() && !states.is_empty() && !states.iter().any(|s| zone.contains(s))
    }

    /// The text names a country other than the one the caller's hint names ("Lisbon, Maine, USA"
    /// for Portugal).
    fn crosses_hint(&self, x: &Components) -> bool {
        let Some(h) = x.cc.as_deref() else {
            return false;
        };
        let named = self.countries_named(x);
        !named.is_empty() && !named.contains(&h)
    }

    /// A two-letter code in the text is a state or province code of a country that writes one after
    /// the town (`POSTAL_CODE_COUNTRIES`) and of no region of the hinted country ("Paris, TX" read
    /// for France). Other countries' two-letter region codes are too common to tell ("KL" is also
    /// Kuala Lumpur written short).
    fn foreign_code(&self, x: &Components) -> bool {
        let Some(h) = x.cc.as_deref() else {
            return false;
        };
        let Some(home) = self.data.countries.get(h) else {
            return false;
        };
        x.localities.iter().any(|t| {
            t.key.len() == 2
                && t.key.bytes().all(|b| b.is_ascii_lowercase())
                && home.region_ids(&t.key).is_empty()
                && POSTAL_CODE_COUNTRIES.iter().any(|&cc| cc != h && self.data.countries.get(cc).map_or(false, |co| co.regions.contains_key(&t.key)))
        })
    }

    /// A town read across scripts from a Chinese romanization ("Fuzhou" for "抚州市") is the place
    /// only where something confirms it: both sides name the same region, or the town is itself a
    /// region of the hinted country under a name no other region shares ("Taichung" for "台中市").
    /// Toneless romanizations name several cities ("Fuzhou": 福州 and 抚州; "Shanxi": 山西 and 陕西).
    fn city_reading_unconfirmed(&self, q: &Components, c: &Components) -> bool {
        let (Some(a), Some(b)) = (&q.city, &c.city) else {
            return false;
        };
        let (lat, han) = if is_latinish(a.script) && fold::all_han(&b.native) {
            (a, b)
        } else if is_latinish(b.script) && fold::all_han(&a.native) {
            (b, a)
        } else {
            return false;
        };
        if q.cc.as_deref() == Some("JP") {
            return false;
        }
        let letters: String = if lat.native.is_empty() { lat.key.clone() } else { lat.native.chars().filter(|ch| ch.is_ascii_alphabetic()).flat_map(|ch| ch.to_lowercase()).collect() };
        let mut stripped: Vec<char> = han.native.chars().collect();
        if stripped.len() >= 2 && HAN_ADMIN.contains(stripped.last().unwrap()) {
            stripped.pop();
        }
        let natives = [han.native.clone(), stripped.into_iter().collect()];
        if !natives.iter().any(|n| !n.is_empty() && reading::reads_as(n, &letters, reading::Lang::Chinese)) {
            return false;
        }
        let (rq, rc) = (self.region_ids(q, false), self.region_ids(c, false));
        if rq.iter().any(|r| rc.iter().any(|x| x.0 == r.0)) {
            return false;
        }
        let Some(co) = c.cc.as_deref().and_then(|cc| self.data.countries.get(cc)) else {
            return true;
        };
        let own = Self::name_regions(co, han);
        let shared = co.region_shared.contains_key(&lat.key) || co.region_shared.contains_key(&letters);
        !(own.len() == 1 && !shared)
    }

    /// The countries a text names, as country codes.
    fn countries_named(&self, x: &Components) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        for k in &x.countries {
            for (cc, co) in &self.data.countries {
                if co.names.iter().any(|n| n == k) && !v.contains(&cc.as_str()) {
                    v.push(cc.as_str());
                }
            }
        }
        v
    }

    /// The two texts name different countries ("United Kingdom" and "Canada").
    fn countries_differ(&self, a: &Components, b: &Components) -> bool {
        let (ca, cb) = (self.countries_named(a), self.countries_named(b));
        !ca.is_empty() && !cb.is_empty() && !ca.iter().any(|c| cb.contains(c))
    }

    /// How a row's area relates to the caller's (or another row's: `rows`). `Fail`: another
    /// postcode, region, country or town, or a city region in place of the caller's town. `Ask`: the
    /// row names no town where the caller gave one, or, with no country to read codes by (or between
    /// two rows), each side gives a place the other does not ("CA" may be California or Canada).
    fn area_fit(&self, qa: &Components, ca: &Components, rows: bool) -> Area {
        if let (Some(a), Some(b)) = (&qa.postcode, &ca.postcode) {
            if a != b {
                return Area::Fail;
            }
        }
        if self.regions_differ(qa, ca) || self.town_region_differs(qa, ca) || self.town_region_differs(ca, qa) || self.countries_differ(qa, ca) || (self.crosses_hint(ca) && !self.crosses_hint(qa)) || (self.foreign_code(ca) && !self.foreign_code(qa)) {
            return Area::Fail;
        }
        // a state written after the caller's town with no comma ("Exampleton IL") that the row
        // leaves out: the row fits, not in full (see `area_unconfirmed`)
        let mut full = (qa.postcode.is_none() || ca.postcode.is_some()) && (qa.town_region.is_none() || ca.town_region == qa.town_region || !self.region_ids(ca, true).is_empty());
        // a town named like a region, with no other region beside it ("Example Park, Texas",
        // "Example Park, Washington"): the row must name it as its town or its region
        if let Some(t) = qa.city.as_ref().filter(|t| qa.regions.contains(&t.key) && self.region_ids(qa, false).is_empty()) {
            // as a town, or as the end of a longer name ("Wilayah Persekutuan Kuala Lumpur")
            let names = |u: &Text| u.key == t.key || (t.key.len() >= 4 && u.key.ends_with(&t.key)) || text_eq(self, t, u, false) == Tri::Yes;
            let as_town = ca.localities.iter().any(|u| !ca.countries.contains(&u.key) && names(u)) || (ca.street_type.is_empty() && !ca.street.is_empty() && names(&ca.street));
            let mine = qa.cc.as_deref().and_then(|cc| self.data.countries.get(cc)).map_or(Vec::new(), |co| Self::name_regions(co, t));
            let row_regions = self.region_ids(ca, true);
            if !as_town && !row_regions.iter().any(|r| mine.contains(&r.0)) {
                // another region is another place; a town alone may be in that region: ask
                return if qa.cc.is_some() && !row_regions.is_empty() { Area::Fail } else { Area::Ask };
            }
            // read by a country the caller did not give, in the region, but the caller's name may be
            // a city ("New York", "Washington", "Ontario" in California) and the row names another
            // town: ask, as a street address would read the city
            if qa.cc_inferred && !as_town && !self.towns(ca).is_empty() {
                return Area::Ask;
            }
        }
        let (qt, ct) = (self.towns(qa), self.towns(ca));
        for t in &qt {
            let named = ca.localities.iter().any(|u| !ca.countries.contains(&u.key) && text_eq(self, t, u, false) == Tri::Yes);
            if named {
                continue;
            }
            // with no country to read it by, the part may be a region or its code ("CA"): ask
            // or a town that is itself a region ("Moscow") against another region the row gives
            let row_regions = self.region_ids(ca, true);
            let mine = qa.cc.as_deref().and_then(|cc| self.data.countries.get(cc)).map_or(Vec::new(), |co| Self::name_regions(co, t));
            let other_region = !mine.is_empty() && !row_regions.is_empty() && !row_regions.iter().any(|r| mine.contains(&r.0));
            if qa.cc.is_some() && (!ct.is_empty() || row_regions.iter().any(|r| r.2) || other_region) {
                return Area::Fail;
            }
            return Area::Ask;
        }
        let (qr, cr) = (self.region_ids(qa, true), self.region_ids(ca, true));
        if qr.iter().any(|r| !cr.iter().any(|x| x.0 == r.0)) {
            full = false;
        }
        let unmatched = |x: &Components, y: &Components| {
            x.localities.iter().any(|t| !y.localities.iter().any(|u| u.key == t.key || text_eq(self, t, u, false) == Tri::Yes)) || x.countries.iter().any(|k| !y.countries.contains(k))
        };
        // two rows that each give a place the other does not may be in two places
        if rows && unmatched(qa, ca) && unmatched(ca, qa) {
            return Area::Ask;
        }
        // with no country to read the caller's area by, a row's part the caller did not give may
        // put it elsewhere ("London, Ontario" for "London, UK"): ask
        if !rows && qa.cc.is_none() && (!qa.localities.is_empty() || !qa.countries.is_empty()) && unmatched(ca, qa) {
            return Area::Ask;
        }
        Area::Fits { full }
    }

    /// Two rows' streets and house numbers may be one place: either gives none, or both give the
    /// same street and no two different house numbers.
    fn streets_agree(&self, a: &Components, b: &Components) -> bool {
        if a.street.is_empty() || b.street.is_empty() {
            return true;
        }
        self.street_eq(a, b) && !matches!((&a.house, &b.house), (Some(x), Some(y)) if x != y)
    }

    /// The same street name and, where both give one, a street type of the same kind.
    fn street_eq(&self, a: &Components, b: &Components) -> bool {
        text_eq(self, &a.street, &b.street, false) == Tri::Yes && (a.street_type.is_empty() || b.street_type.is_empty() || intersects(&a.street_type, &b.street_type))
    }

    /// Choose among rows for a caller who named a place. The same name in the caller's area is the
    /// place; a variant of the name asks; a row that names the place and then a street is still the
    /// place ("Example Park, 12 Sample Plaza"). A row that does not state the caller's town or street
    /// asks. Same-name rows in two towns, or on two streets (two branches), are two places, so the
    /// answer asks. A same-name row next to a variant asks too: the variant may be the place under its
    /// full name, and a read may take the one of them the site categorized.
    fn named_decide(&self, ctx: &Ctx, query: &str, name: &[(String, Vec<u32>)], rows: &[Row]) -> (Pick, Explained) {
        let rest = |s: &str| -> String { s.splitn(2, ',').nth(1).unwrap_or("").to_string() };
        let qa = Self::area_parse(ctx, &rest(query));
        let caller_street = !qa.street.is_empty() && (qa.house.is_some() || !qa.street_type.is_empty());
        // a town typed alone ("Munich") is compared as a town, with its other-language names
        let town_alone = if query.contains(',') { None } else { Some(parse::parse(ctx, query)).filter(|q| q.street.is_empty() && q.house.is_none() && q.city.is_some()) };
        let report = |verdict: Verdict, reasons: Vec<&'static str>| RowReport { verdict, reasons, notes: vec![] };
        let mut reports: Vec<RowReport> = Vec::with_capacity(rows.len());
        let mut exact: Vec<(usize, bool, Components)> = Vec::new();
        let mut variant: Vec<usize> = Vec::new();
        let mut variant_area: Vec<(bool, Components)> = Vec::new();
        let mut unconfirmed: Vec<usize> = Vec::new();
        // a longer-name row that may be another place named after the caller's
        let mut named_after_any = false;
        // a row of the caller's exact name outside the caller's area
        let mut exact_elsewhere = false;
        let query_codes = self.short_codes(query.split(',').next().unwrap_or(""));
        let mut codes_of: Vec<Vec<String>> = vec![Vec::new(); rows.len()];
        for (i, row) in rows.iter().enumerate() {
            if self.is_control(query, row.text) {
                reports.push(report(Verdict::Different, vec!["control"]));
                continue;
            }
            let (c, part_of_place) = self.named_row(ctx, row.text, name, &qa);
            let c = &*c;
            if part_of_place {
                reports.push(report(Verdict::Different, vec!["place_name"]));
                continue;
            }
            let first = c.split(',').next().unwrap_or("");
            let words = self.name_words(&self.without_short_codes(first));
            // a row whose first part names a venue by a place-type word the caller did not write
            // ("International Terminal, Example International Airport" for "Example International
            // Airport") is that venue, not a place in the caller's town
            let venue_part = words.iter().any(|w| !name.iter().any(|n| n.0 == w.0) && self.data.lookup(&w.0).iter().any(|e| e.class == data::Class::Place));
            // a town typed alone, with no country: a row adding a place the caller did not name may
            // be another town of the name ("Paris, TX" for "Paris")
            if let Some(q) = town_alone.as_ref().filter(|q| q.cc.is_none() && !venue_part) {
                let whole = parse::parse(ctx, c);
                let named = |u: &Text| q.localities.iter().any(|t| t.key == u.key || text_eq(self, t, u, false) == Tri::Yes);
                if whole.street.is_empty() && whole.house.is_none() && whole.localities.iter().any(named) && whole.localities.iter().any(|u| !whole.countries.contains(&u.key) && !named(u)) {
                    reports.push(report(Verdict::Unknown, vec!["place_unconfirmed"]));
                    unconfirmed.push(i);
                    continue;
                }
            }
            let mut rel = self.name_relation(name, &words);
            // two short codes that differ name two places ("(EXA)" and "(EXB)")
            let row_codes = self.short_codes(first);
            codes_of[i] = row_codes.clone();
            if !query_codes.is_empty() && !row_codes.is_empty() && !row_codes.iter().any(|k| query_codes.contains(k)) {
                rel = 0;
            }
            if rel == 0 && !venue_part {
                if let Some(q) = &town_alone {
                    let whole = parse::parse(ctx, c);
                    // with no allowance for a typo: "Homburg" is not "Hamburg"
                    let (v, why) = if whole.street.is_empty() && whole.house.is_none() { self.compare_parsed(q, &whole, Mode::Peer) } else { (Verdict::Different, vec![]) };
                    if v == Verdict::Same {
                        reports.push(report(Verdict::Same, vec![]));
                        exact.push((i, true, whole));
                        continue;
                    }
                    if why == ["city_reading"] {
                        reports.push(report(Verdict::Unknown, vec!["place_unconfirmed"]));
                        unconfirmed.push(i);
                        continue;
                    }
                }
            }
            if rel == 0 {
                reports.push(report(Verdict::Different, vec!["place_name"]));
                continue;
            }
            let ca = Self::area_parse(ctx, &rest(c));
            let mut area = self.area_fit(&qa, &ca, false);
            // a street the caller typed after the name ("Oak St", "12 Oak"; a lone word may be a town)
            // must be the row's street
            if caller_street && area != Area::Fail {
                if ca.street.is_empty() {
                    area = Area::Ask;
                } else if !self.street_eq(&qa, &ca) || matches!((&qa.house, &ca.house), (Some(x), Some(y)) if x != y) {
                    reports.push(report(Verdict::Different, vec!["street_name"]));
                    continue;
                }
            }
            match area {
                Area::Fail => {
                    exact_elsewhere |= rel == 2;
                    reports.push(report(Verdict::Different, vec!["area"]));
                    continue;
                }
                Area::Ask => {
                    reports.push(report(Verdict::Unknown, vec!["place_unconfirmed"]));
                    unconfirmed.push(i);
                    rel = 0;
                }
                Area::Fits { .. } => {}
            }
            let full = matches!(area, Area::Fits { full: true });
            if rel == 2 {
                reports.push(report(Verdict::Same, vec![]));
                exact.push((i, full, ca));
            } else if rel == 1 {
                reports.push(report(Verdict::Unknown, vec!["place_name_variant"]));
                variant.push(i);
                variant_area.push((full, ca));
                named_after_any |= self.named_after(name, &words);
            }
        }
        let asked = |exact: &[(usize, bool, Components)], extra: Vec<&'static str>, reports: Vec<RowReport>| {
            let mut ids: Vec<usize> = exact.iter().map(|e| e.0).chain(variant.iter().copied()).chain(unconfirmed.iter().copied()).collect();
            ids.sort();
            let mut reasons = extra;
            if !variant.is_empty() {
                reasons.push("place_name_variant");
            }
            if !unconfirmed.is_empty() {
                reasons.push("place_unconfirmed");
            }
            (Pick::Ambiguous(ids.clone()), Explained::ambiguous(ids, reasons, reports))
        };
        // a read may take the venue the site categorized only when the caller gave a town or
        // postcode and the venue holds it in full; with no area it asks
        let caller_area = qa.postcode.is_some() || !self.towns(&qa).is_empty();
        if exact.is_empty() {
            if variant.is_empty() && unconfirmed.is_empty() {
                return (Pick::None, Explained::none(reports));
            }
            let (pick, mut e) = asked(&exact, vec![], reports);
            // the exact name only outside the caller's area, beside one categorized longer-name row
            // that holds it in full
            if exact_elsewhere && caller_area && unconfirmed.is_empty() && !named_after_any && variant.len() == 1 && variant_area[0].0 && rows[variant[0]].categorized {
                e.prefer_on_read = Some(variant[0]);
            }
            return (pick, e);
        }
        // same-name rows in two towns, or on two streets, are two places; more distinct rows than
        // `MAX_COMPARED_ROWS` are not compared with each other, but asked about
        let one_place = |a: &Components, b: &Components| matches!(self.area_fit(a, b, true), Area::Fits { .. }) && matches!(self.area_fit(b, a, true), Area::Fits { .. }) && self.streets_agree(a, b);
        let crowded = distinct_texts(exact.iter().map(|e| e.0).chain(variant.iter().copied()).map(|i| rows[i].text)) > MAX_COMPARED_ROWS;
        // rows of one text are one place: each text is compared once
        let keys: Vec<String> = rows.iter().map(|r| text_key(r.text)).collect();
        let first_of_text = |i: usize| keys.iter().position(|k| *k == keys[i]) == Some(i);
        let towns_apart = crowded || {
            let reps: Vec<&(usize, bool, Components)> = exact.iter().filter(|e| first_of_text(e.0)).collect();
            // two rows whose short codes differ ("(EXA)" and "(EXB)") are two places
            let codes_apart = |a: usize, b: usize| !codes_of[a].is_empty() && !codes_of[b].is_empty() && !codes_of[a].iter().any(|k| codes_of[b].contains(k));
            reps.iter().any(|a| reps.iter().any(|b| a.0 != b.0 && (!one_place(&a.2, &b.2) || codes_apart(a.0, b.0))))
        };
        if towns_apart || !variant.is_empty() || !unconfirmed.is_empty() {
            let (pick, mut e) = asked(&exact, if towns_apart { vec!["several_places"] } else { vec![] }, reports);
            // the venue the site categorized, beside a same-name row it did not (or the reverse), when
            // the caller gave a town or postcode, every row holds the caller's area in full, and no
            // longer-name row may be another place named after the caller's (a read with no area
            // asks)
            let areas: Vec<(bool, &Components)> = exact.iter().filter(|x| first_of_text(x.0)).map(|x| (x.1, &x.2)).chain(variant.iter().zip(&variant_area).filter(|(&i, _)| first_of_text(i)).map(|(_, x)| (x.0, &x.1))).collect();
            let has_town = |x: &Components| !self.towns(x).is_empty();
            let one_town = !crowded && caller_area && areas.iter().all(|(_, a)| has_town(a));
            // the venue holds the caller's area in full; the other rows fit it, if not in full (a pin
            // with no postcode for "Example Center, 94000")
            let full_at = |i: usize| exact.iter().find(|x| x.0 == i).map(|x| x.1).or_else(|| variant.iter().position(|&v| v == i).map(|k| variant_area[k].0)).unwrap_or(false);
            if !towns_apart && unconfirmed.is_empty() && one_town && !named_after_any {
                if let Pick::Ambiguous(ids) = &pick {
                    let categorized: Vec<usize> = ids.iter().copied().filter(|&i| rows[i].categorized).collect();
                    if categorized.len() == 1 && full_at(categorized[0]) {
                        e.prefer_on_read = Some(categorized[0]);
                    }
                }
            }
            return (pick, e);
        }
        // a row of the caller's exact name the site categorized beside one it did not (a map pin
        // dropped under the venue's own name): they may be two places, so a write asks; a read may
        // take the categorized one when the caller gave a town or postcode it holds in full. (`pick`
        // sees no categories: unmarked rows of one name in one town are one place.)
        let distinct: Vec<&(usize, bool, Components)> = exact.iter().filter(|x| first_of_text(x.0)).collect();
        let venues: Vec<&&(usize, bool, Components)> = distinct.iter().filter(|x| rows[x.0].categorized).collect();
        if !venues.is_empty() && venues.len() < distinct.len() {
            let (pick, mut e) = asked(&exact, vec!["several_places"], reports);
            if caller_area && venues.len() == 1 && venues[0].1 {
                e.prefer_on_read = Some(venues[0].0);
            }
            return (pick, e);
        }
        // the site's first row that holds every area part the caller gave, else its first
        let best = exact.iter().find(|e| e.1).unwrap_or(&exact[0]).0;
        let first = |s: &str| fold::dominant_script(s.split(',').next().unwrap_or(""));
        let cross = script_group(Some(first(query))) != script_group(Some(first(rows[best].text)));
        let mut e = Explained { outcome: Outcome::Match, index: Some(best), same: exact.iter().map(|x| x.0).collect(), cross_script: cross, ..Explained::none(reports) };
        if cross {
            e.rows[best].reasons.push("cross_script");
        }
        (Pick::Match(best), e)
    }

    /// A rejected candidate that may be the same street and number in another place, rejected for a
    /// reason that depends on fragile parsing. Picking would then risk a wrong pick: report ambiguity.
    fn has_shadow<S: AsRef<str>>(&self, q: &Components, cs: &[Components], cluster: &[usize], raw: &[S]) -> bool {
        let hn = match &q.house {
            Some(h) => h.clone(),
            None => return false,
        };
        let digits: String = hn.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return false;
        }
        for (i, c) in cs.iter().enumerate() {
            if cluster.contains(&i) {
                continue;
            }
            let (v, r) = self.compare_parsed(q, c, Mode::Query);
            if v == Verdict::Same {
                continue;
            }
            let fragile = r.iter().any(|x| matches!(*x, "street_name" | "po_box_missing" | "street_missing" | "street_name_not_comparable" | "city_not_comparable" | "candidate_has_no_house_number" | "house_number"));
            if !fragile {
                continue;
            }
            // a row that opens with what may be a unit ("Bay 12, 12 Example St") is read as that unit
            let unit_first = self.without_leading_name(raw[i].as_ref()).map_or(false, |(rest, name)| {
                let next = fold::fold(rest.split(',').next().unwrap_or(""));
                name.as_deref().and_then(Self::maybe_unit).is_some() && next.split(|ch: char| !ch.is_ascii_digit()).any(|x| x == digits)
            });
            if unit_first {
                continue;
            }
            // the raw text must hold the caller's number as a whole digit run
            let text = fold::fold(raw[i].as_ref());
            let runs: Vec<&str> = text.split(|ch: char| !ch.is_ascii_digit()).filter(|x| !x.is_empty()).collect();
            if !runs.contains(&digits.as_str()) {
                continue;
            }
            if r.contains(&"house_number") && c.house.as_deref().map_or(false, |h| h.chars().take_while(|ch| ch.is_ascii_digit()).collect::<String>() != digits) && c.house.is_some() {
                // a real different number parsed cleanly (e.g. "10" vs "100") unless the caller's number is elsewhere
                if c.postcode.as_deref() != Some(&digits.to_ascii_uppercase()) && !c.units.iter().any(|u| u[1..] == digits) {
                    continue;
                }
            }
            let qs = &q.street.key;
            let ck = text.chars().filter(|ch| ch.is_alphanumeric()).collect::<String>();
            let ck = fold::ascii_key(&ck);
            let stem: String = qs.chars().take(4).collect();
            let not_comparable = r.iter().any(|x| x.ends_with("not_comparable")) && c.house.as_deref() == Some(hn.as_str());
            if !not_comparable && (stem.len() < 3 || !ck.contains(&stem)) {
                continue;
            }
            // and it would be a different place than the cluster's pick
            let j = cluster[0];
            if not_comparable || self.compare_parsed(c, &cs[j], Mode::Peer).0 == Verdict::Different || self.compare_parsed(&cs[j], c, Mode::Peer).0 == Verdict::Different {
                if c.city.is_some() && cs[j].city.is_some() && !matches!(text_eq(self, c.city.as_ref().unwrap(), cs[j].city.as_ref().unwrap(), false), Tri::Yes) {
                    return true;
                }
                if c.postcode.is_some() && cs[j].postcode.is_some() && c.postcode != cs[j].postcode {
                    return true;
                }
            }
        }
        false
    }

    /// `x`'s street words start with `y`'s whole street name and the words after it name `y`'s town:
    /// return `x` with those words moved to its localities.
    /// A region name of the country the text was read for ("dolnoslaskie", "emiliaromagna").
    fn is_region(&self, x: &Components, key: &str) -> bool {
        x.cc.as_deref().and_then(|cc| self.data.countries.get(cc)).map_or(false, |co| co.regions.contains_key(key))
    }

    /// One caller typo in a compound street word: "keelo" + "polku" vs "kielo" + "polku".
    fn compound_typo(&self, q: &Components, c: &Components) -> bool {
        let full = |x: &Components| -> String { if x.street_type_compound && !x.street_type_first { format!("{}{}", x.street.key, x.street_type_word) } else { x.street.key.clone() } };
        let same_type = (q.street_type.is_empty() && c.street_type.is_empty()) || intersects(&q.street_type, &c.street_type) || q.street_type.is_empty() || c.street_type.is_empty();
        (q.street_type_compound || c.street_type_compound) && same_type && typo_in_long_word(&[full(q).as_str()], &full(c))
    }

    /// A hyphenated house number on the listing may be read as a range: the caller's country is
    /// given and on `RANGE_COUNTRIES`, both sides are in Latin script, and, in the US, neither side is
    /// in Hawaii (ZIPs 967xx and 968xx, zone-and-house numbers) or Queens-style numbering (ZIPs 110xx
    /// to 116xx, block-and-lot numbers).
    fn range_reading(q: &Components, c: &Components) -> bool {
        let Some(cc) = q.cc.as_deref() else {
            return false;
        };
        if !RANGE_COUNTRIES.contains(&cc) || script_group(q.script) != 0 || script_group(c.script) != 0 {
            return false;
        }
        if cc == "US" {
            for x in [q, c] {
                let zip3: String = x.postcode.as_deref().unwrap_or("").chars().take(3).collect();
                if matches!(zip3.as_str(), "967" | "968" | "110" | "111" | "112" | "113" | "114" | "115" | "116") {
                    return false;
                }
                if x.regions.iter().any(|r| r == "hi" || r == "hawaii") || x.localities.iter().any(|l| l.key == "queens") {
                    return false;
                }
            }
        }
        true
    }

    /// `x` with house "73k" when `y` has house "73" and a street starting with the word "k":
    /// `x` with the letter moved to the front of its street.
    fn letter_to_street(x: &Components, y: &Components) -> Option<Components> {
        let (xh, yh) = (x.house.as_ref()?, y.house.as_ref()?);
        let l = xh.strip_prefix(yh.as_str())?;
        if l.chars().count() != 1 || !l.chars().all(|ch| ch.is_alphabetic()) || y.street.toks.first().map(|t| t.0.as_str()) != Some(l) {
            return None;
        }
        if x.street.toks.first().map(|t| t.0.as_str()) == Some(l) {
            return None;
        }
        let mut x2 = x.clone();
        x2.house = Some(yh.clone());
        x2.street.toks.insert(0, (l.to_string(), vec![]));
        x2.street.abbr.insert(0, false);
        x2.street.key = x2.street.toks.iter().map(|t| t.0.as_str()).collect();
        x2.street.skel = fold::skeleton(&x2.street.key);
        Some(x2)
    }

    /// `Err(true)`: the street matched but the split-off town is not one of `y`'s.
    fn realign(&self, x: &Components, y: &Components, typo: bool) -> Result<Components, bool> {
        let k = y.street.toks.len();
        if k == 0 || x.street.toks.len() <= k || y.localities.is_empty() {
            return Err(false);
        }
        let mk = |toks: &[(String, Vec<u32>)], abbr: &[bool], sc: Option<Script>| -> Text {
            let key: String = toks.iter().map(|t| t.0.as_str()).collect();
            Text { skel: fold::skeleton(&key), key, script: sc, toks: toks.to_vec(), abbr: abbr.to_vec(), native: String::new() }
        };
        let head = mk(&x.street.toks[..k], &x.street.abbr[..k.min(x.street.abbr.len())], x.street.script);
        // the other side may carry its type inside the name ("reunalan" + "tie")
        let y_full = if y.street_type_compound && !y.street_type_first { Some(format!("{}{}", y.street.key, y.street_type_word)) } else { None };
        let full_eq = y_full.as_deref().map_or(false, |f| f == head.key || (typo && typo_in_long_word(&[head.key.as_str()], f)));
        // or abbreviate its type inside the word ("Tuomarink." for "Tuomarinkatu")
        let compound_ok = || {
            let mut x3 = x.clone();
            x3.street = head.clone();
            self.compound_eq(&x3, y)
        };
        if text_eq(self, &head, &y.street, typo) != Tri::Yes && !full_eq && !compound_ok() {
            return Err(false);
        }
        let mut rest: Vec<(String, Vec<u32>)> = x.street.toks[k..].to_vec();
        // "504 Independence St Berkeley Springs": the type word read last ("Springs") belongs to the
        // town, and the street's own type is the first word after the name
        let mut new_type: Option<(Vec<u32>, String, bool)> = None;
        if !x.street_type_word.is_empty() && !x.street_type_first && !x.street_type_compound && !y.street_type.is_empty() {
            let first_is_type = rest.first().map_or(false, |t| t.0 == y.street_type_word || self.data.lookup(&t.0).iter().any(|e| e.class == data::Class::StreetType && y.street_type.contains(&e.canon)));
            if first_is_type {
                let t = rest.remove(0);
                let canons: Vec<u32> = self.data.lookup(&t.0).iter().filter(|e| e.class == data::Class::StreetType).map(|e| e.canon).collect();
                let abbr = x.street.abbr.get(k).copied().unwrap_or(false);
                rest.push((x.street_type_word.clone(), vec![]));
                new_type = Some((canons, t.0, abbr));
            }
        }
        if rest.is_empty() {
            return Err(false);
        }
        // a trailing country name is not part of the town
        while rest.len() > 1 && self.data.is_country_name(&rest.last().unwrap().0) {
            rest.pop();
        }
        // "Higuera de la Sierra": articles may be kept on one side and dropped on the other
        let long = |t: &Text| -> String { t.toks.iter().filter(|w| w.0.chars().count() > 2 || w.0.chars().any(|ch| ch.is_ascii_digit())).map(|w| w.0.as_str()).collect() };
        // the words may name one town or several places in a row ("Canosa di Puglia" read as
        // "Canosa" + "Puglia"): cover them with the other side's place names, longest first
        let mut places: Vec<Text> = Vec::new();
        let mut i = 0;
        while i < rest.len() {
            let mut found = None;
            for j in (i + 1..=rest.len()).rev() {
                let t = mk(&rest[i..j], &vec![false; j - i], x.street.script);
                if let Some(l) = y.localities.iter().find(|l| text_eq(self, &t, l, typo) == Tri::Yes || long(&t) == long(l)) {
                    found = Some((j, Some(l.clone())));
                    break;
                }
                if i > 0 && (y.regions.contains(&t.key) || self.data.is_country_name(&t.key) || self.is_region(x, &t.key)) {
                    found = Some((j, None));
                    break;
                }
            }
            match found {
                Some((j, l)) => {
                    places.extend(l);
                    i = j;
                }
                None => return Err(true),
            }
        }
        let mut x2 = x.clone();
        x2.street = head;
        if let Some((canons, word, abbr)) = new_type {
            x2.street_type = canons;
            x2.street_type_word = word;
            x2.street_type_abbr = abbr;
            x2.street_type_guess = false;
        }
        for (n, l) in places.iter().enumerate() {
            x2.localities.insert(n, l.clone());
        }
        x2.city = places.first().cloned();
        Ok(x2)
    }

    /// Whole street strings (name with its type word in written order) equal, within one typo for
    /// the caller, or with equal consonant skeletons across scripts: "Avnida Peralta" vs
    /// "Avenida Peralta", "Tryq Fhd" vs "طريق فهد".
    fn full_street_eq(&self, q: &Components, c: &Components, typo: bool) -> bool {
        if q.street.is_empty() || c.street.is_empty() {
            return false;
        }
        // with a type on both sides, the ordinary name + type comparison decides
        if !q.street_type.is_empty() && !c.street_type.is_empty() {
            return false;
        }
        let full = |x: &Components| -> Vec<String> {
            let mut v = vec![x.street.key.clone()];
            if !x.street_type_word.is_empty() {
                v.push(if x.street_type_first { format!("{}{}", x.street_type_word, x.street.key) } else { format!("{}{}", x.street.key, x.street_type_word) });
            }
            v
        };
        // a typo inside the caller's street type ("Avnida", "Hiighway") hides the type
        if typo && q.street_type.is_empty() && !c.street_type.is_empty() && q.street.toks.len() >= 2 {
            let n = q.street.toks.len();
            for (edge, rest) in [(0usize, 1..n), (n - 1, 0..n - 1)] {
                let t = &q.street.toks[edge].0;
                if letters(t) < 5 {
                    continue;
                }
                let near = c.street_type.iter().any(|cid| self.data.type_keys_by_canon.get(cid).map_or(false, |ws| ws.iter().any(|w| w.len() >= 5 && dl_le1(t, w))));
                if near {
                    let restk: String = q.street.toks[rest].iter().map(|x| x.0.as_str()).collect();
                    if restk == c.street.key {
                        return true;
                    }
                }
            }
        }
        let (qs, cs) = (full(q), full(c));
        let cross = is_latinish(q.street.script) != is_latinish(c.street.script);
        for a in &qs {
            for b in &cs {
                if a == b && (a != &q.street.key || b != &c.street.key) {
                    return true;
                }
                if typo && (a != &q.street.key || b != &c.street.key) && {
                    let mut tk: Vec<&str> = q.street.toks.iter().map(|t| t.0.as_str()).collect();
                    if a != &q.street.key {
                        if q.street_type_first {
                            tk.insert(0, q.street_type_word.as_str());
                        } else {
                            tk.push(q.street_type_word.as_str());
                        }
                    }
                    typo_in_long_word(&tk, b)
                } {
                    return true;
                }
                if cross && a.len() >= 4 && fold::skeleton(a) == fold::skeleton(b) && !fold::skeleton(a).is_empty() {
                    return true;
                }
            }
        }
        false
    }

    /// `x` writes the street as one word ("rasmussenplads", "johanssonv.") that is `y`'s name plus
    /// `y`'s street type (same type, a synonym, or an abbreviation of it).
    fn compound_eq(&self, x: &Components, y: &Components) -> bool {
        if y.street_type.is_empty() || y.street.is_empty() || x.street.is_empty() {
            return false;
        }
        if !x.street_type.is_empty() && !x.street_type_compound {
            return false;
        }
        let mut xs: Vec<String> = vec![x.street.key.clone()];
        if x.street_type_compound {
            xs = vec![format!("{}{}", x.street.key, x.street_type_word)];
        }
        let x_last_abbr = x.street.abbr.last().copied().unwrap_or(false);
        for xk in xs {
            if xk.len() <= y.street.key.len() {
                continue;
            }
            // the type sits after the name ("knutsengata") or before it ("naberezhnaiapromyshlennaia")
            let t = if xk.starts_with(&y.street.key) && !y.street_type_first {
                &xk[y.street.key.len()..]
            } else if xk.ends_with(&y.street.key) && y.street_type_first {
                &xk[..xk.len() - y.street.key.len()]
            } else {
                continue;
            };
            let tw = &y.street_type_word;
            // a dictionary abbreviation names its own types only ("k." is katu, not kuja)
            if t == x.street_type_word && !x.street_type.is_empty() && !intersects(&x.street_type, &y.street_type) && !x.street_type_guess && !y.street_type_guess {
                // unless the dictionary lists it for the other type too ("v." for veg, vei and veien)
                let lists = |ids: &[u32], w0: &str| self.data.lookup(w0).iter().any(|e| ids.contains(&e.canon));
                let listed = lists(&y.street_type, t) || lists(&x.street_type, tw);
                if !listed {
                    continue;
                }
            }
            if t == tw {
                return true;
            }
            // a dictionary word or abbreviation on either side ("kg", "g.") stands only for its own types
            let in_dict = |w: &str| self.data.lookup(w).iter().any(|e| e.class == data::Class::StreetType);
            if (y.street_type_abbr && !y.street_type_guess && in_dict(tw)) || in_dict(t) {
                if self.data.lookup(t).iter().any(|e| y.street_type.contains(&e.canon)) {
                    return true;
                }
                // a Nordic definite form of the abbreviated type ("gt." for "gatan")
                let (short, long) = if t.len() < tw.len() { (t, tw.as_str()) } else { (tw.as_str(), t) };
                let definite = self.data.lookup(short).iter().filter(|e| e.class == data::Class::StreetType).any(|e| {
                    let ck = &self.data.canon_keys[e.canon as usize];
                    NORDIC_TYPES.contains(&ck.as_str()) && long.len() > ck.len() && long.starts_with(ck.as_str()) && matches!(&long[ck.len()..], "en" | "n" | "et" | "t" | "an" | "a" | "e" | "na" | "ne")
                });
                if definite {
                    return true;
                }
                continue;
            }
            if (y.street_type_abbr && parse::is_abbrev(tw, t)) || ((t.len() >= 2 || x_last_abbr) && parse::is_abbrev(t, tw)) {
                return true;
            }
            if self.data.lookup(t).iter().any(|e| y.street_type.contains(&e.canon)) {
                return true;
            }
            for cid in &y.street_type {
                if let Some(keys) = self.data.type_keys_by_canon.get(cid) {
                    if keys.iter().any(|w| w == t || (x_last_abbr && parse::is_abbrev(t, w))) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Is the row in the caller's area, whatever place it is? The caller gave a town, region or
    /// postcode after the first part, and the row, after its own first part, shows no other postcode
    /// and either the caller's postcode or each town and region the caller gave. A picker that must
    /// read something can take such a row when no row is the caller's place.
    pub fn in_area(&self, query: &str, row: &str, country: Option<&str>) -> bool {
        let ((query, ql), (row, rl)) = (self.bound(query), self.bound(row));
        if ql || rl {
            return false;
        }
        let (query, row) = (&*query, &*row);
        let ctx = self.ctx(country);
        let rest = |s: &str| -> String { s.splitn(2, ',').nth(1).unwrap_or("").to_string() };
        let (qr, rr) = (rest(query), rest(row));
        if qr.trim().is_empty() || rr.trim().is_empty() {
            return false;
        }
        let qa = Self::area_parse(&ctx, &qr);
        let ra = Self::area_parse(&ctx, &rr);
        if qa.postcode.is_some() && qa.postcode == ra.postcode && ra.postcode.is_some() {
            return true;
        }
        if qa.localities.is_empty() && qa.regions.is_empty() {
            return false;
        }
        // with no town given, the caller's first part (the place, or a town) must be in the row
        // ("New York, NY" and "Washington, DC" name a town: a town named like its region, with the
        // region after it)
        if self.towns(&qa).iter().all(|t| self.data.is_country_name(&t.key)) {
            let name: Vec<String> = self.name_words(query.split(',').next().unwrap_or("")).into_iter().map(|w| w.0).collect();
            let r: Vec<String> = self.name_words(&rr).into_iter().map(|w| w.0).collect();
            if name.is_empty() || !r.windows(name.len()).any(|w| w == &name[..]) {
                return false;
            }
        }
        matches!(self.area_fit(&qa, &ra, false), Area::Fits { full: true })
    }

    /// Choose among a site's rows, saying why: the outcome, every row that is the caller's place
    /// (so a picker may choose by the site's own order among rows of one place), each row's verdict
    /// and reasons, the reasons an `ambiguous` answer asks, notes beside a `match`, and the row a read
    /// may take when a write asks. Unlike `pick`, it also asks where the only rows near the caller's
    /// place show another postcode or leave out the house number, and it reads a listing's
    /// house-number range where one is meant (see `RANGE_COUNTRIES`).
    pub fn pick_explained(&self, query: &str, rows: &[Row], options: &PickOptions) -> Explained {
        self.pick_explained_with(query, rows, options, &Rules::default())
    }

    /// `pick_explained` under optional `rules`, each off by default (see `Rules`).
    pub fn pick_explained_with(&self, query: &str, rows: &[Row], options: &PickOptions, rules: &Rules) -> Explained {
        /// Turns the rule off when the pick ends, a panic caught by its caller included.
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                DISTRICT_MAY_DIFFER.with(|cell| cell.set(false));
            }
        }
        DISTRICT_MAY_DIFFER.with(|cell| cell.set(rules.district_may_differ));
        let _reset = Reset;
        self.pick_explained_inner(query, rows, options)
    }

    fn pick_explained_inner(&self, query: &str, rows: &[Row], options: &PickOptions) -> Explained {
        let (query, query_lost) = self.bound(query);
        let query = &*query;
        let cut: Vec<(std::borrow::Cow<str>, bool)> = rows.iter().map(|r| self.bound(r.text)).collect();
        let lost: Vec<bool> = cut.iter().map(|c| c.1).collect();
        let rows: Vec<Row> = rows.iter().zip(&cut).map(|(r, t)| Row { text: &t.0, categorized: r.categorized }).collect();
        let rows = rows.as_slice();
        let texts: Vec<&str> = rows.iter().map(|r| r.text).collect();
        let given = self.country_code(options.country);
        let inferred = if given.is_none() { self.infer_country(query, &texts) } else { None };
        let country = given.or(inferred.as_deref());
        let (p, e) = self.decide_home(query, rows, country, inferred.is_some(), true);
        let mut e = Self::unless_cut(p, e, &lost, query_lost).1;
        e.unit = self.unit_as_written(query, country);
        e.inferred_country = inferred;
        e
    }

    /// Choose the candidate that is the same place as the query. It takes no categories, so a
    /// picker that knows which rows the site shows as venues should use `pick_explained`.
    pub fn pick<S: AsRef<str>>(&self, query: &str, candidates: &[S], country: Option<&str>) -> Pick {
        let (query, query_lost) = self.bound(query);
        let query = &*query;
        let cut: Vec<(std::borrow::Cow<str>, bool)> = candidates.iter().map(|c| self.bound(c.as_ref())).collect();
        let lost: Vec<bool> = cut.iter().map(|c| c.1).collect();
        let rows: Vec<Row> = cut.iter().map(|c| Row::from(&*c.0)).collect();
        let texts: Vec<&str> = rows.iter().map(|r| r.text).collect();
        let country = self.country_code(country);
        let inferred = if country.is_none() { self.infer_country(query, &texts) } else { None };
        let (p, e) = self.decide_home(query, &rows, country.or(inferred.as_deref()), inferred.is_some(), false);
        Self::unless_cut(p, e, &lost, query_lost).0
    }

    /// `pick`'s answer and `pick_explained`'s; `ranges`: read listings' house-number ranges.
    /// `inferred`: the country was read from the texts, not given.
    fn decide_read(&self, query: &str, rows: &[Row], country: Option<&str>, inferred: bool, ranges: bool) -> (Pick, Explained) {
        let mut ctx = self.ctx(country);
        ctx.inferred = inferred && ctx.cc.is_some();
        let q = parse::parse(&ctx, query);
        if let Some(name) = self.place_name(query, &q) {
            return self.named_decide(&ctx, query, &name, rows);
        }
        // a place's name and a postcode alone ("Example Center, 94000", "Example Center 94000") is
        // the name with the postcode as its area, not a house number
        // (read as a street address first: "Example River Rd 15202" may be a number after its
        // street, so the name reading is used only when the address reading takes no row and the
        // name reading finds one)
        let named_postcode = Self::name_and_postcode(query).and_then(|named| {
            let area = Self::area_parse(&ctx, named.splitn(2, ',').nth(1).unwrap_or(""));
            let name = self.name_words(&self.without_short_codes(named.split(',').next().unwrap_or("")));
            (area.postcode.is_some() && q.po_box.is_none() && q.units.is_empty() && !name.is_empty()).then(|| self.named_decide(&ctx, &named, &name, rows))
        });
        let street = self.decide_street(query, &ctx, &q, rows, ranges);
        match named_postcode {
            Some(named) if !matches!(street.0, Pick::Match(_)) && !matches!(named.0, Pick::None) => named,
            _ => street,
        }
    }

    /// `decide_read` for a street address.
    fn decide_street(&self, query: &str, ctx: &Ctx, q: &Components, rows: &[Row], ranges: bool) -> (Pick, Explained) {
        let (ctx, q) = (ctx, q.clone());
        let candidates: Vec<&str> = rows.iter().map(|r| r.text).collect();
        // each listing as compared, without a distance it opens with
        let compared: Vec<std::borrow::Cow<str>> = candidates.iter().map(|c| Self::without_distance(c)).collect();
        let mut cs: Vec<Components> = compared.iter().map(|c| self.parse_row(ctx, query, &q, c)).collect();
        let alts: Vec<Components> = Self::comma_free_readings(query).iter().map(|a| parse::parse(&ctx, a)).collect();
        let calts: Vec<Option<(Components, Option<String>)>> = compared.iter().map(|c| self.without_leading_name(c).map(|(t, name)| (parse::parse(&ctx, &t), name))).collect();
        // a site's control ("See more results for <text>") is never a place, nor a row to ask about
        let mut verdicts: Vec<(Verdict, Vec<&'static str>)> = cs
            .iter()
            .zip(&calts)
            .zip(&candidates)
            .map(|((c, ca), text)| if self.is_control(query, text) { (Verdict::Different, vec!["control"]) } else { self.compare_any(query, &q, &alts, c, ca.as_ref()) })
            .collect();
        let mut reports: Vec<RowReport> = verdicts.iter().map(|(v, r)| RowReport { verdict: *v, reasons: r.clone(), notes: vec![] }).collect();
        // a listing taken by its address after a leading name ("24 Hour Sample, 12 Example St") is
        // read as that address from here on (a row that only looks like the caller's place is still
        // checked against the listing as written)
        let as_written = cs.clone();
        let mut rewritten = vec![false; cs.len()];
        for i in 0..cs.len() {
            if let (Verdict::Same, Some((c2, _))) = (verdicts[i].0, &calts[i]) {
                if self.compare_parsed(&q, &cs[i], Mode::Query).0 != Verdict::Same && self.compare_parsed(&q, c2, Mode::Query).0 == Verdict::Same {
                    cs[i] = c2.clone();
                    rewritten[i] = true;
                }
            }
        }
        // a listing's range holding the caller's number ("10-14" for 12) is read as that number: the
        // caller's building when both sides give the same postcode, asked about otherwise
        let mut ranged = vec![false; cs.len()];
        let listed_house: Vec<Option<String>> = cs.iter().map(|c| c.house.clone()).collect();
        if let (true, Some(qh)) = (ranges, &q.house) {
            for i in 0..cs.len() {
                let holds = verdicts[i].1 == ["house_number"] && cs[i].house.as_deref().map_or(false, |h| range_holds(h, qh)) && Self::range_reading(&q, &cs[i]);
                if !holds {
                    continue;
                }
                let mut c2 = cs[i].clone();
                c2.house = Some(qh.clone());
                let (v, r) = self.compare_parsed(&q, &c2, Mode::Query);
                let confirmed = q.postcode.is_some() && q.postcode == c2.postcode;
                let read: (Verdict, Vec<&'static str>) = match v {
                    Verdict::Same if confirmed => (Verdict::Same, vec!["house_number_range"]),
                    Verdict::Same => (Verdict::Unknown, vec!["number_range"]),
                    // the row leaves out the caller's town or state: it stays a question
                    Verdict::Unknown if r == ["town_not_listed"] || r == ["region_not_listed"] => (Verdict::Unknown, vec!["number_range"]),
                    Verdict::Different if r == ["postcode"] => (Verdict::Different, vec!["postcode"]),
                    _ => continue,
                };
                let mut shown = read.1.clone();
                if !shown.contains(&"house_number_range") && !shown.contains(&"number_range") {
                    shown.push("house_number_range");
                }
                reports[i] = RowReport { verdict: read.0, reasons: shown, notes: vec![] };
                verdicts[i] = read;
                cs[i] = c2;
                ranged[i] = true;
            }
        }
        let same: Vec<usize> = (0..cs.len()).filter(|&i| verdicts[i].0 == Verdict::Same).collect();
        if same.is_empty() {
            // the caller gave no unit and the site lists the place only with units: ask which one
            let with_unit: Vec<usize> = (0..cs.len()).filter(|&i| matches!(verdicts[i].1.as_slice(), ["candidate_adds_unit"] | ["leading_unit"])).collect();
            let strict = if with_unit.is_empty() { Pick::None } else { Pick::Ambiguous(with_unit) };
            // rows near the caller's place: a picker asks about them
            let mut near: Vec<usize> = Vec::new();
            let mut reasons: Vec<&'static str> = Vec::new();
            for (i, (v, r)) in verdicts.iter().enumerate() {
                let why = match (v, r.as_slice()) {
                    (_, ["candidate_adds_unit"] | ["leading_unit"]) => "unit_not_given",
                    (Verdict::Unknown, ["candidate_has_no_house_number"]) => "number_not_listed",
                    (Verdict::Unknown, ["number_range"]) => "number_range",
                    (Verdict::Unknown, ["town_not_listed"] | ["region_not_listed"] | ["town_spelled_differently"]) => "place_unconfirmed",
                    (Verdict::Different, ["postcode"]) if self.postcode_only_differs(&q, &cs[i]) => "postcode_differs",
                    _ => continue,
                };
                near.push(i);
                reasons.push(why);
                if !reports[i].reasons.contains(&why) {
                    reports[i].reasons.push(why);
                }
            }
            return if near.is_empty() { (strict, Explained::none(reports)) } else { (strict, Explained::ambiguous(near, reasons, reports)) };
        }
        // a row that opens with what may be a unit before the caller's address may be the caller's
        // place too: the rows are asked about, so neither is taken over the other
        // (a row with the caller's own unit is still taken: "Unit 5, 12 Example St" for Unit 5)
        let lead: Vec<usize> = (0..cs.len()).filter(|&i| verdicts[i].1 == ["leading_unit"]).collect();
        let unit_rows = !q.units.is_empty() && same.iter().all(|&i| !cs[i].units.is_empty());
        if !lead.is_empty() && !unit_rows {
            let mut ids: Vec<usize> = same.iter().chain(&lead).copied().collect();
            ids.sort();
            for &i in &lead {
                reports[i].reasons.push("unit_not_given");
            }
            return (Pick::Ambiguous(ids.clone()), Explained::ambiguous(ids, vec!["several_places", "unit_not_given"], reports));
        }
        // the caller's place names a row keeps, and those it adds that the caller did not write
        let named = |t: &Text| q.localities.iter().any(|u| text_eq(self, u, t, false) == Tri::Yes);
        let kept = |i: usize| cs[i].localities.iter().filter(|t| named(t)).count();
        let added = |i: usize| cs[i].localities.len() - kept(i);
        // the same address in a town that opens with the caller's town and goes on
        // ("Exampleton-North" or "Exampleton Beach" for "Exampleton") may be the caller's town
        // with a district, or another town: when every row of the caller's place adds a place the
        // caller did not write, the rows are asked about, so such a row is never passed over unseen
        let opens_town = |i: usize| {
            let (Some(qc), Some(rc)) = (&q.city, &cs[i].city) else {
                return false;
            };
            let first = rc.toks.first().map(|t| t.0.as_str()).unwrap_or("");
            let first = self.data.exonyms.get(first).map_or(first, |c| c.as_str());
            rc.toks.len() >= 2 && qc.toks.len() == 1 && !first.is_empty() && first == qc.key
        };
        let opening: Vec<usize> = (0..cs.len()).filter(|&i| verdicts[i].0 != Verdict::Same && verdicts[i].1 == ["city"] && opens_town(i)).collect();
        if !opening.is_empty() && same.iter().all(|&i| added(i) > 0) {
            let mut ids: Vec<usize> = same.iter().chain(&opening).copied().collect();
            ids.sort();
            for &i in &opening {
                reports[i].reasons.push("place_unconfirmed");
            }
            return (Pick::Ambiguous(ids.clone()), Explained::ambiguous(ids, vec!["place_unconfirmed"], reports));
        }
        // more distinct rows of the caller's place than `MAX_COMPARED_ROWS` are not compared with
        // each other, but asked about
        if distinct_texts(same.iter().map(|&i| candidates[i])) > MAX_COMPARED_ROWS {
            return (Pick::Ambiguous(same.clone()), Explained::ambiguous(same, vec!["several_places"], reports));
        }
        // cluster into distinct places; rows of one text are one place, so each cluster is compared
        // through one row of each of its texts
        let keys: Vec<String> = candidates.iter().map(|t| text_key(t)).collect();
        let mut clusters: Vec<Vec<usize>> = Vec::new();
        let mut reps: Vec<Vec<usize>> = Vec::new();
        for &i in &same {
            let mut placed = false;
            for (cl, rp) in clusters.iter_mut().zip(reps.iter_mut()) {
                // two different listed ranges holding the caller's number are two claims, not one place
                let ranges_differ = |j: usize| ranged[i] && ranged[j] && listed_house[i] != listed_house[j];
                let one = |j: usize| self.compare_parsed(&cs[i], &cs[j], Mode::Peer).0 != Verdict::Different && self.compare_parsed(&cs[j], &cs[i], Mode::Peer).0 != Verdict::Different;
                let known = rp.iter().any(|&j| keys[j] == keys[i] && !ranges_differ(j));
                if known || rp.iter().all(|&j| !ranges_differ(j) && one(j)) {
                    cl.push(i);
                    if !known {
                        rp.push(i);
                    }
                    placed = true;
                    break;
                }
            }
            if !placed {
                clusters.push(vec![i]);
                reps.push(vec![i]);
            }
        }
        if clusters.len() > 1 {
            // the caller's own words decide between places: keep the one place whose town or
            // postcode the caller wrote ("... byt 13 Beroun-Město" with no commas)
            let raw: String = fold::fold(query).chars().filter(|ch| ch.is_alphanumeric()).collect();
            let raw = fold::ascii_key(&raw);
            let score = |cl: &Vec<usize>| -> usize {
                cl.iter()
                    .map(|&i| {
                        let town = cs[i].city.as_ref().map_or(false, |t| t.key.len() >= 3 && raw.contains(&t.key) && !q.street.key.contains(&t.key) && !cs[i].street.key.contains(&t.key));
                        let pc = cs[i].postcode.as_ref().map_or(false, |p| {
                            let p = p.to_ascii_lowercase();
                            p.len() >= 4 && raw.contains(&p) && !q.house.as_ref().map_or(false, |h| h.contains(&p))
                        });
                        town as usize + 2 * pc as usize
                    })
                    .max()
                    .unwrap_or(0)
            };
            let scores: Vec<usize> = clusters.iter().map(score).collect();
            let top = *scores.iter().max().unwrap();
            if top > 0 && scores.iter().filter(|&&x| x == top).count() == 1 {
                let k = scores.iter().position(|&x| x == top).unwrap();
                clusters = vec![clusters[k].clone()];
            }
        }
        if clusters.len() > 1 {
            return (Pick::Ambiguous(same.clone()), Explained::ambiguous(same, vec!["several_places"], reports));
        }
        let mut place = clusters[0].clone();
        place.sort();
        if self.has_shadow(&q, &as_written, &clusters[0], &compared) {
            return (Pick::Ambiguous(place.clone()), Explained::ambiguous(place, vec!["several_places"], reports));
        }
        // rows that leave out the caller's state, beside the same address in another state: they
        // may be that row, so ask ("1 Example St, Exampleton" beside "..., Exampleton, WI")
        if self.state_left_out_beside_another(&q, &cs, &place, &verdicts) {
            for &i in &place {
                reports[i].reasons.push("region_not_listed");
            }
            return (Pick::Ambiguous(place.clone()), Explained::ambiguous(place, vec!["place_unconfirmed"], reports));
        }
        // prefer the listing with the caller's own number over a range holding it, a listing that is
        // the caller's place as written over one read without its leading name, the listing without
        // a business name, then the most complete one
        // (the most complete: the most of the caller's own place names, then the fewest the caller
        // did not name, so a row that adds a town or district is not taken over one that adds none)
        let best = clusters[0].iter().copied().min_by_key(|&i| (cs[i].units != q.units, ranged[i], rewritten[i], cs[i].business.len(), usize::MAX - kept(i), added(i))).unwrap();
        // a row read across scripts: its street or town is written in another script than the caller's
        let across = |i: usize| {
            let (c, qq) = (&cs[i], &q);
            (!qq.street.is_empty() && !c.street.is_empty() && script_group(qq.street.script) != script_group(c.street.script))
                || matches!((&qq.city, &c.city), (Some(a), Some(b)) if script_group(a.script) != script_group(b.script))
        };
        // a row that leaves out a unit the caller gave
        let unit_left_out = |i: usize| q.units.iter().any(|u| !cs[i].units.iter().any(|v| v[1..] == u[1..]));
        for &i in &place {
            if unit_left_out(i) {
                reports[i].notes.push("unit_not_listed");
            }
            if across(i) {
                reports[i].reasons.push("cross_script");
            }
        }
        let notes = if unit_left_out(best) { vec!["unit_not_listed"] } else { vec![] };
        let e = Explained { outcome: Outcome::Match, index: Some(best), same: place, notes, cross_script: across(best), ..Explained::none(reports) };
        (Pick::Match(best), e)
    }
}

/// A Danish postal district's letters after its town's name, as their ASCII keys ("Ø" is "o"):
/// the same town when the other side leaves them out, or writes the town as its municipality.
const DANISH_DISTRICTS: &[&str] = &["c", "k", "v", "n", "s", "o", "m", "nv", "no", "sv", "so"];

/// The US and the territories its postal service serves: `Rules::district_may_differ` never
/// applies there.
const US_POSTAL: &[&str] = &["US", "PR", "GU", "VI", "AS", "MP", "UM"];

/// The Danish towns whose postal districts are written with letters after the town's name, as
/// their ASCII keys, before and after exonyms.
const DANISH_DISTRICT_TOWNS: &[&str] = &["kobenhavn", "copenhagen", "frederiksberg", "aarhus", "arhus", "odense", "aalborg", "alborg", "esbjerg", "randers", "vejle"];
