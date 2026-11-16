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
        let ctx = self.ctx(country);
        let q = parse::parse(&ctx, query);
        let c = parse::parse(&ctx, candidate);
        self.compare_parsed(&q, &c, Mode::Query)
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

    /// Choose the candidate that is the same place as the query.
    pub fn pick<S: AsRef<str>>(&self, query: &str, candidates: &[S], country: Option<&str>) -> Pick {
        let ctx = self.ctx(country);
        let q = parse::parse(&ctx, query);
        let cs: Vec<Components> = candidates.iter().map(|c| parse::parse(&ctx, c.as_ref())).collect();
        let verdicts: Vec<(Verdict, Vec<&'static str>)> = cs.iter().map(|c| self.compare_parsed(&q, c, Mode::Query)).collect();
        let same: Vec<usize> = (0..cs.len()).filter(|&i| verdicts[i].0 == Verdict::Same).collect();
        if same.is_empty() {
            // the caller gave no unit and the site lists the place only with units: ask which one
            let with_unit: Vec<usize> = (0..cs.len()).filter(|&i| verdicts[i].1 == ["candidate_adds_unit"]).collect();
            if !with_unit.is_empty() {
                return Pick::Ambiguous(with_unit);
            }
            return Pick::None;
        }
        // cluster into distinct places
        let mut clusters: Vec<Vec<usize>> = Vec::new();
        for &i in &same {
            let mut placed = false;
            for cl in clusters.iter_mut() {
                if cl.iter().all(|&j| self.compare_parsed(&cs[i], &cs[j], Mode::Peer).0 != Verdict::Different && self.compare_parsed(&cs[j], &cs[i], Mode::Peer).0 != Verdict::Different) {
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
        if clusters.len() == 1 && self.has_shadow(&q, &cs, &clusters[0], candidates) {
            let mut ids = clusters[0].clone();
            ids.sort();
            return Pick::Ambiguous(ids);
        }
        if clusters.len() == 1 {
            // prefer the listing without a business name, then the most complete one
            let best = clusters[0].iter().copied().min_by_key(|&i| (cs[i].units != q.units, cs[i].business.len(), usize::MAX - cs[i].localities.len())).unwrap();
            Pick::Match(best)
        } else {
            Pick::Ambiguous(same)
        }
    }
}
