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

/// Options for `Matcher::pick_explained`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PickOptions<'a> {
    /// The site's country (ISO 3166-1 alpha-2), when known.
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
///   ("CA" may be California or Canada);
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
    /// them the site categorized, when exactly one is and the rows name one town (the caller's, when
    /// the caller gave an area). A read may take it; a write still asks.
    pub prefer_on_read: Option<usize>,
    /// Each row's verdict and reasons, by row index.
    pub rows: Vec<RowReport>,
}

impl Explained {
    fn none(rows: Vec<RowReport>) -> Explained {
        Explained { outcome: Outcome::None, index: None, same: vec![], reasons: vec![], notes: vec![], cross_script: false, prefer_on_read: None, rows }
    }

    fn ambiguous(same: Vec<usize>, reasons: Vec<&'static str>, rows: Vec<RowReport>) -> Explained {
        let mut reasons = reasons;
        reasons.sort_by_key(|r| AMBIGUITY_REASONS.iter().position(|x| x == r));
        reasons.dedup();
        Explained { outcome: Outcome::Ambiguous, index: None, same, reasons, notes: vec![], cross_script: false, prefer_on_read: None, rows }
    }

    /// The answer as one line of JSON: `outcome` ("match", "none", "ambiguous"), `index`, `same`,
    /// `reasons`, `notes`, `cross_script`, `prefer_on_read` and `rows` (each `verdict` and `reasons`).
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
        format!(
            "{{\"outcome\":\"{}\",\"index\":{},\"same\":[{}],\"reasons\":{},\"notes\":{},\"cross_script\":{},\"prefer_on_read\":{},\"rows\":[{}]}}",
            outcome,
            opt(self.index),
            self.same.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","),
            words(&self.reasons),
            words(&self.notes),
            self.cross_script,
            opt(self.prefer_on_read),
            rows.join(",")
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
const CONTROL_WORDS: &[&str] = &["see", "search", "searching", "show", "more", "results", "result", "use", "find", "view", "browse", "everything", "matches", "suggestions", "explore"];
/// Words a control row may hold besides control words and counts.
const CONTROL_FILLERS: &[&str] = &["for", "all", "in", "of", "the", "my", "a", "an", "to", "with", "about", "other", "on", "near"];
/// Cities that repeat street names across districts and postcodes.
const REPEATED_STREET_CITIES: &[&str] = &["berlin", "hamburg", "london", "wien", "vienna", "budapest", "praha", "prague"];

/// How a row's area relates to the caller's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Area {
    Fail,
    Ask,
    /// `full`: the row holds every area part the caller gave
    Fits { full: bool },
}

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

/// Compare two names (street or locality). `typo`: allow one edit for names of 6+ letters.
fn text_eq(m: &Matcher, q: &Text, c: &Text, typo: bool) -> Tri {
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

    fn ctx<'a>(&'a self, hint: Option<&'a str>) -> Ctx<'a> {
        let cc = hint.map(|h| h.trim()).filter(|h| self.data.countries.contains_key(*h));
        let langs = cc.and_then(|c| self.data.countries.get(c)).map(|c| c.langs.as_slice()).filter(|l| !l.is_empty());
        Ctx { data: &self.data, langs, cc, cjk: &self.cjk }
    }

    /// Parse an address into the components used for matching.
    pub fn normalize(&self, text: &str, country: Option<&str>) -> Components {
        parse::parse(&self.ctx(country), text)
    }

    /// Is the candidate the same place as the query? Returns the verdict and the reasons.
    pub fn compare(&self, query: &str, candidate: &str, country: Option<&str>) -> (Verdict, Vec<&'static str>) {
        // a site's control ("See more results for <text>") is never a place
        if self.is_control(query, candidate) {
            return (Verdict::Different, vec!["control"]);
        }
        let ctx = self.ctx(country);
        let q = parse::parse(&ctx, query);
        if let Some(name) = self.place_name(query, &q) {
            return match self.named_decide(&ctx, query, &name, &[Row::from(candidate)]).0 {
                Pick::Match(_) => (Verdict::Same, vec![]),
                Pick::Ambiguous(_) => (Verdict::Unknown, vec!["place_name_variant"]),
                Pick::None => (Verdict::Different, vec!["place_name"]),
            };
        }
        let c = parse::parse(&ctx, candidate);
        let alts: Vec<Components> = Self::comma_free_readings(query).iter().map(|a| parse::parse(&ctx, a)).collect();
        let calt = Self::without_leading_name(candidate).map(|t| parse::parse(&ctx, &t));
        self.compare_any(&q, &alts, &c, calt.as_ref())
    }

    /// A listing that starts with a business or building name holding digits ("Grand 2009 Ltd,
    /// Main St 6, Town"): the listing without that first part, so its digits are not read as the
    /// house number.
    fn without_leading_name(candidate: &str) -> Option<String> {
        let parts: Vec<&str> = candidate.split(',').collect();
        if parts.len() < 3 || !parts[0].chars().any(|ch| ch.is_numeric()) || !parts[1].chars().any(|ch| ch.is_numeric()) {
            return None;
        }
        Some(parts[1..].join(","))
    }

    /// The query as read, or as one of its comma-free readings if that reading is the candidate's
    /// place.
    fn compare_any(&self, q: &Components, alts: &[Components], c: &Components, calt: Option<&Components>) -> (Verdict, Vec<&'static str>) {
        let first = self.compare_parsed(q, c, Mode::Query);
        if first.0 == Verdict::Same {
            return first;
        }
        if let Some(c2) = calt {
            if c2.house != c.house && q.house.is_some() && c2.house == q.house {
                let r = self.compare_parsed(q, c2, Mode::Query);
                if r.0 == Verdict::Same {
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
        if self.regions_differ(q, c) {
            return (Verdict::Different, vec!["region"]);
        }
        if self.countries_differ(q, c) {
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
            if !any_yes {
                if any_unknown {
                    unknown = true;
                    reasons.push("city_not_comparable");
                } else {
                    return (Verdict::Different, vec!["city"]);
                }
            } else if !peer {
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
        let words = self.name_words(first);
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
        if a.len() != b.len() || a.len() < 3 || a[..a.len() - 2] != b[..b.len() - 2] {
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
        if REPEATED_STREET_CITIES.contains(&qc.key.as_str()) && qt.len() < 2 {
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

    /// A text's place names that are neither a region nor a country.
    fn towns(&self, x: &Components) -> Vec<Text> {
        x.localities.iter().filter(|t| !x.regions.contains(&t.key) && !self.data.is_country_name(&t.key)).cloned().collect()
    }

    /// The regions a text names (indexes in its country's `region_info`), each with the lists that
    /// name it and whether it is a city: its regions, and place names that are a region's short name
    /// ("Taichung", "Jiangsu").
    fn region_ids(&self, x: &Components) -> Vec<(u16, u8, bool)> {
        let Some(co) = x.cc.as_deref().and_then(|c| self.data.countries.get(c)) else {
            return Vec::new();
        };
        let mut v: Vec<(u16, u8, bool)> = x
            .regions
            .iter()
            .filter_map(|k| co.regions.get(k))
            .chain(x.localities.iter().filter_map(|t| co.regions.get(&t.key).or_else(|| co.region_short.get(&t.key))))
            .map(|&id| {
                let info = co.region_info.get(id as usize).copied().unwrap_or_default();
                (id, info.sources, info.city)
            })
            .collect();
        v.sort();
        v.dedup();
        v
    }

    /// The two texts name different regions of one level ("IL" and "Missouri", "Jiangsu" and
    /// "福建省", "Taichung" and "台南市").
    fn regions_differ(&self, a: &Components, b: &Components) -> bool {
        let (ra, rb) = (self.region_ids(a), self.region_ids(b));
        [1u8, 2].iter().any(|&bit| {
            let xa: Vec<u16> = ra.iter().filter(|r| r.1 & bit != 0).map(|r| r.0).collect();
            let xb: Vec<u16> = rb.iter().filter(|r| r.1 & bit != 0).map(|r| r.0).collect();
            !xa.is_empty() && !xb.is_empty() && !xa.iter().any(|i| xb.contains(i))
        })
    }

    /// The countries a text names, as country codes.
    fn countries_named(&self, x: &Components) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        for k in x.countries.iter().chain(x.localities.iter().map(|t| &t.key).filter(|k| self.data.is_country_name(k))) {
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
        if self.regions_differ(qa, ca) || self.countries_differ(qa, ca) {
            return Area::Fail;
        }
        let mut full = qa.postcode.is_none() || ca.postcode.is_some();
        let (qt, ct) = (self.towns(qa), self.towns(ca));
        for t in &qt {
            let named = ca.localities.iter().any(|u| !self.data.is_country_name(&u.key) && text_eq(self, t, u, false) == Tri::Yes);
            if named {
                continue;
            }
            // with no country to read it by, the part may be a region or its code ("CA"): ask
            if qa.cc.is_some() && (!ct.is_empty() || self.region_ids(ca).iter().any(|r| r.2)) {
                return Area::Fail;
            }
            return Area::Ask;
        }
        let (qr, cr) = (self.region_ids(qa), self.region_ids(ca));
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
        for (i, row) in rows.iter().enumerate() {
            let c = row.text;
            if self.is_control(query, c) {
                reports.push(report(Verdict::Different, vec!["control"]));
                continue;
            }
            let first = c.split(',').next().unwrap_or("");
            let words = self.name_words(first);
            let mut rel = self.name_relation(name, &words);
            if rel == 0 {
                if let Some(q) = &town_alone {
                    let whole = parse::parse(ctx, c);
                    if whole.street.is_empty() && whole.house.is_none() && self.compare_parsed(q, &whole, Mode::Query).0 == Verdict::Same {
                        reports.push(report(Verdict::Same, vec![]));
                        exact.push((i, true, whole));
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
        if exact.is_empty() {
            return if variant.is_empty() && unconfirmed.is_empty() { (Pick::None, Explained::none(reports)) } else { asked(&exact, vec![], reports) };
        }
        // same-name rows in two towns, or on two streets, are two places
        let one_place = |a: &Components, b: &Components| matches!(self.area_fit(a, b, true), Area::Fits { .. }) && matches!(self.area_fit(b, a, true), Area::Fits { .. }) && self.streets_agree(a, b);
        let towns_apart = exact.iter().any(|a| exact.iter().any(|b| a.0 != b.0 && !one_place(&a.2, &b.2)));
        if towns_apart || !variant.is_empty() || !unconfirmed.is_empty() {
            let (pick, mut e) = asked(&exact, if towns_apart { vec!["several_places"] } else { vec![] }, reports);
            // the venue the site categorized, beside a same-name row it did not (or the reverse), when
            // the rows name one town (and one region, when the caller gave no area), the caller's area
            // in full when the caller gave one, and no longer-name row may be another place named
            // after the caller's
            let areas: Vec<(bool, &Components)> = exact.iter().map(|x| (x.1, &x.2)).chain(variant_area.iter().map(|x| (x.0, &x.1))).collect();
            let has_town = |x: &Components| !self.towns(x).is_empty();
            let caller_area = qa.postcode.is_some() || !qa.localities.is_empty() || !qa.regions.is_empty();
            let one_region = |a: &Components, b: &Components| caller_area || (!self.region_ids(a).is_empty() && self.region_ids(a).iter().any(|r| self.region_ids(b).iter().any(|x| x.0 == r.0)));
            let one_town = areas.iter().all(|(full, a)| has_town(a) && (*full || !caller_area) && areas.iter().all(|(_, b)| one_place(a, b) && one_region(a, b)));
            if !towns_apart && unconfirmed.is_empty() && one_town && !named_after_any {
                if let Pick::Ambiguous(ids) = &pick {
                    let categorized: Vec<usize> = ids.iter().copied().filter(|&i| rows[i].categorized).collect();
                    if categorized.len() == 1 {
                        e.prefer_on_read = Some(categorized[0]);
                    }
                }
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
        if qa.localities.iter().all(|t| qa.regions.contains(&t.key) || self.data.is_country_name(&t.key)) {
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
        self.decide(query, rows, options.country, true).1
    }

    /// Choose the candidate that is the same place as the query.
    pub fn pick<S: AsRef<str>>(&self, query: &str, candidates: &[S], country: Option<&str>) -> Pick {
        let rows: Vec<Row> = candidates.iter().map(|c| Row::from(c.as_ref())).collect();
        self.decide(query, &rows, country, false).0
    }

    /// `pick`'s answer and `pick_explained`'s; `ranges`: read listings' house-number ranges.
    fn decide(&self, query: &str, rows: &[Row], country: Option<&str>, ranges: bool) -> (Pick, Explained) {
        let ctx = self.ctx(country);
        let q = parse::parse(&ctx, query);
        if let Some(name) = self.place_name(query, &q) {
            return self.named_decide(&ctx, query, &name, rows);
        }
        let candidates: Vec<&str> = rows.iter().map(|r| r.text).collect();
        let mut cs: Vec<Components> = candidates.iter().map(|c| parse::parse(&ctx, c)).collect();
        let alts: Vec<Components> = Self::comma_free_readings(query).iter().map(|a| parse::parse(&ctx, a)).collect();
        let calts: Vec<Option<Components>> = candidates.iter().map(|c| Self::without_leading_name(c).map(|t| parse::parse(&ctx, &t))).collect();
        // a site's control ("See more results for <text>") is never a place, nor a row to ask about
        let mut verdicts: Vec<(Verdict, Vec<&'static str>)> = cs
            .iter()
            .zip(&calts)
            .zip(&candidates)
            .map(|((c, ca), text)| if self.is_control(query, text) { (Verdict::Different, vec!["control"]) } else { self.compare_any(&q, &alts, c, ca.as_ref()) })
            .collect();
        let mut reports: Vec<RowReport> = verdicts.iter().map(|(v, r)| RowReport { verdict: *v, reasons: r.clone(), notes: vec![] }).collect();
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
            let with_unit: Vec<usize> = (0..cs.len()).filter(|&i| verdicts[i].1 == ["candidate_adds_unit"]).collect();
            let strict = if with_unit.is_empty() { Pick::None } else { Pick::Ambiguous(with_unit) };
            // rows near the caller's place: a picker asks about them
            let mut near: Vec<usize> = Vec::new();
            let mut reasons: Vec<&'static str> = Vec::new();
            for (i, (v, r)) in verdicts.iter().enumerate() {
                let why = match (v, r.as_slice()) {
                    (_, ["candidate_adds_unit"]) => "unit_not_given",
                    (Verdict::Unknown, ["candidate_has_no_house_number"]) => "number_not_listed",
                    (Verdict::Unknown, ["number_range"]) => "number_range",
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
        // cluster into distinct places
        let mut clusters: Vec<Vec<usize>> = Vec::new();
        for &i in &same {
            let mut placed = false;
            for cl in clusters.iter_mut() {
                // two different listed ranges holding the caller's number are two claims, not one place
                let ranges_differ = |j: usize| ranged[i] && ranged[j] && listed_house[i] != listed_house[j];
                if cl.iter().all(|&j| !ranges_differ(j) && self.compare_parsed(&cs[i], &cs[j], Mode::Peer).0 != Verdict::Different && self.compare_parsed(&cs[j], &cs[i], Mode::Peer).0 != Verdict::Different) {
                    cl.push(i);
                    placed = true;
                    break;
                }
            }
            if !placed {
                clusters.push(vec![i]);
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
        if self.has_shadow(&q, &cs, &clusters[0], &candidates) {
            return (Pick::Ambiguous(place.clone()), Explained::ambiguous(place, vec!["several_places"], reports));
        }
        // prefer the listing with the caller's own number over a range holding it, the listing
        // without a business name, then the most complete one
        let best = clusters[0].iter().copied().min_by_key(|&i| (cs[i].units != q.units, ranged[i], cs[i].business.len(), usize::MAX - cs[i].localities.len())).unwrap();
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
