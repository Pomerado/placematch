//! Lightweight component tagger: enough structure to decide whether two strings name one place.
use crate::data::{Class, Data, Entry};
use crate::fold::{ascii_key, dominant_script, fold, script_of, skeleton, Script};
use crate::token::{tokenize, CjkRules};

#[derive(Clone, Debug, Default)]
pub struct Text {
    /// ASCII key (transliterated, concatenated, no spaces)
    pub key: String,
    pub skel: String,
    pub script: Option<Script>,
    /// per-token keys and canonical ids (for token-wise comparison)
    pub toks: Vec<(String, Vec<u32>)>,
    /// per-token: written as an abbreviation
    pub abbr: Vec<bool>,
    /// the folded native-script text of the name, for reading comparisons ("台中")
    pub native: String,
}

impl Text {
    pub fn is_empty(&self) -> bool {
        self.key.is_empty()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Components {
    pub house: Option<String>,
    pub street: Text,
    pub street_type: Vec<u32>,
    pub street_type_script: Option<Script>,
    /// ASCII key of the street type as written ("gt", "vagen")
    pub street_type_word: String,
    /// the type was split off a compound word ("hauptstrasse")
    pub street_type_compound: bool,
    /// the type was written abbreviated with a dot ("Vic.", "Chem.")
    pub street_type_abbr: bool,
    /// the type was guessed from an abbreviation not in the dictionaries
    pub street_type_guess: bool,
    /// the type is written before the name ("Rue X", "ул. X")
    pub street_type_first: bool,
    /// characters of a compound type split off the name's last word
    pub street_type_word_chars: usize,
    pub units: Vec<String>,
    /// the folded words of each unit with its keyword, as the text gives them ("apt 3", "# 4b"), so
    /// the unit can be found as written
    pub unit_words: Vec<String>,
    pub po_box: Option<String>,
    pub postcode: Option<String>,
    pub city: Option<Text>,
    pub localities: Vec<Text>,
    pub regions: Vec<String>,
    pub country: bool,
    /// the country names the text gives ("uk", "canada")
    pub countries: Vec<String>,
    pub business: Vec<Text>,
    pub script: Option<Script>,
    /// the country hint the text was read with
    pub cc: Option<String>,
    /// the hint was read from the texts, not given by the caller
    pub cc_inferred: bool,
    /// a region of the country written at the end of the town's part, with no comma
    /// ("Exampleton IL", "Exampleton New York"): its key
    pub town_region: Option<String>,
    /// the number of comma parts with words in them
    pub parts: usize,
    /// a second number written in the house number's part with no words ("18" in "30 18,
    /// Exampleton"): when both sides give one, another is another place
    pub second_number: Option<String>,
}

pub struct Ctx<'a> {
    pub data: &'a Data,
    pub langs: Option<&'a [u8]>,
    pub cc: Option<&'a str>,
    pub cjk: &'a CjkRules,
    /// `cc` was read from the texts, not given by the caller
    pub inferred: bool,
}

#[derive(Clone, Debug)]
struct Span {
    start: usize,
    len: usize,
    entries: Vec<Entry>,
}

impl Span {
    fn has(&self, c: Class) -> bool {
        self.entries.iter().any(|e| e.class == c)
    }
    fn canons(&self, c: Class) -> Vec<u32> {
        let mut v: Vec<u32> = self.entries.iter().filter(|e| e.class == c).map(|e| e.canon).collect();
        v.sort();
        v.dedup();
        v
    }
}

const NAME_CLASSES: &[Class] = &[Class::Synonym, Class::Directional, Class::Title, Class::StreetType, Class::Qualifier, Class::Building];

fn lookup(ctx: &Ctx, key: &str) -> Vec<Entry> {
    let es = ctx.data.lookup(key);
    if es.is_empty() {
        return Vec::new();
    }
    es.iter()
        .filter(|e| match ctx.langs {
            Some(ls) => ls.contains(&e.lang),
            None => true,
        })
        .copied()
        .collect()
}

/// `t` abbreviates `w`: same first letter, letters of `t` in order inside `w`, shorter.
pub fn is_abbrev(t: &str, w: &str) -> bool {
    let tc: Vec<char> = t.chars().collect();
    let wc: Vec<char> = w.chars().collect();
    if tc.is_empty() || tc.len() >= wc.len() || tc[0] != wc[0] {
        return false;
    }
    let mut j = 0;
    for &c in &wc {
        if j < tc.len() && c == tc[j] {
            j += 1;
        }
    }
    j == tc.len()
}

fn abbrev_type(ctx: &Ctx, t: &str) -> Vec<u32> {
    let first = match t.chars().next() {
        Some(f) => f,
        None => return Vec::new(),
    };
    let mut v: Vec<u32> = ctx
        .data
        .type_by_first
        .get(&first)
        .map(|v| v.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter(|(w, e)| ctx.langs.map_or(true, |ls| ls.contains(&e.lang)) && is_abbrev(t, w))
        .map(|(_, e)| e.canon)
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Longest known suffix of `surf` (char length >= 1) leaving at least `min_rest` chars.
fn find_suffix(ctx: &Ctx, surf: &str, min_rest: usize, only_types: bool) -> Option<(String, Entry)> {
    let chars: Vec<char> = surf.chars().collect();
    let tl = chars.len();
    let maxl = ctx.data.max_suffix.min(tl.saturating_sub(min_rest));
    for l in (1..=maxl).rev() {
        let sfx: String = chars[tl - l..].iter().collect();
        if let Some(es) = ctx.data.suffix_map.get(&sfx) {
            for e in es {
                if only_types && e.class != Class::StreetType {
                    continue;
                }
                if ctx.langs.map_or(true, |ls| ls.contains(&e.lang)) || only_types {
                    return Some((sfx, *e));
                }
            }
        }
    }
    None
}

/// "167th", "2nd", "3e", "1er", "5o" -> digits; other tokens unchanged.
fn ordinal_digits(s: &str) -> String {
    let d: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if d.is_empty() || d.len() == s.len() {
        return s.to_string();
    }
    let rest = &s[d.len()..];
    if matches!(rest, "st" | "nd" | "rd" | "th" | "e" | "er" | "re" | "eme" | "o" | "a" | "oji" | "asis" | "oe" | "de" | "ste" | "te") {
        d
    } else {
        s.to_string()
    }
}

/// An English ordinal word in a street's name, as its number ("Fifth" for "5th"), where streets
/// are written in English or the country is not known.
fn english_ordinal(ctx: &Ctx, s: &str) -> String {
    const WORDS: [&str; 20] = [
        "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth", "eleventh", "twelfth", "thirteenth", "fourteenth", "fifteenth", "sixteenth", "seventeenth", "eighteenth", "nineteenth", "twentieth",
    ];
    let english = matches!(ctx.cc, None | Some("US" | "CA" | "GB" | "IE" | "AU" | "NZ"));
    match WORDS.iter().position(|w| *w == s) {
        Some(i) if english => (i + 1).to_string(),
        _ => s.to_string(),
    }
}

fn norm_number(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '/' {
            out.push(c);
        } else if c.is_alphabetic() {
            // "172б" and "172b" are the same number
            out.push_str(&ascii_key(&c.to_string()));
        }
    }
    out.trim_matches('-').to_string()
}

/// Join a compass pair written with dots ("N.W.", "s. e.") into one word ("NW"), as it is written
/// without them.
fn join_dotted_compass(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains('.') {
        return std::borrow::Cow::Borrowed(text);
    }
    let ch: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut changed = false;
    while i < ch.len() {
        let starts = i == 0 || !ch[i - 1].is_alphanumeric();
        if starts && matches!(ch[i], 'N' | 'n' | 'S' | 's') && ch.get(i + 1) == Some(&'.') {
            let mut j = i + 2;
            while ch.get(j) == Some(&' ') {
                j += 1;
            }
            let second = ch.get(j).copied();
            let ends = ch.get(j + 2).map_or(true, |c| !c.is_alphanumeric());
            if matches!(second, Some('E' | 'e' | 'W' | 'w')) && ch.get(j + 1) == Some(&'.') && ends {
                out.push(ch[i]);
                out.push(second.unwrap());
                i = j + 2;
                changed = true;
                continue;
            }
        }
        out.push(ch[i]);
        i += 1;
    }
    if changed { std::borrow::Cow::Owned(out) } else { std::borrow::Cow::Borrowed(text) }
}

/// Drop a parenthesized article moved behind a name ("Constitución (de la)", "Nacional 110 (la)").
fn drop_paren_articles(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains('(') {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(a) = rest.find('(') {
        let Some(b) = rest[a..].find(')').map(|b| a + b) else { break };
        let inner = &rest[a + 1..b];
        let words: Vec<&str> = inner.split_whitespace().collect();
        let article = !words.is_empty() && words.len() <= 3 && words.iter().all(|w| w.chars().count() <= 3 && w.chars().all(|ch| ch.is_alphabetic() || ch == '\''));
        out.push_str(&rest[..a]);
        if !article {
            out.push_str(&rest[a..=b]);
        }
        rest = &rest[b + 1..];
    }
    out.push_str(rest);
    std::borrow::Cow::Owned(out)
}

/// Thai: split the type and admin prefixes that are written joined to the name ("ถนนติวานนท์"),
/// drop admin words on both sides ("ตำบล", "Subdistrict"), and drop silent letters under the
/// cancellation mark ("นท์" is read "n"), which romanizations leave out.
fn thai_fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // a consonant (and a vowel mark on it) followed by the cancellation mark is silent
        if i + 1 < chars.len() && chars[i + 1] == '\u{0E4C}' {
            i += 2;
            continue;
        }
        if i + 2 < chars.len() && chars[i + 2] == '\u{0E4C}' && ('\u{0E31}'..='\u{0E3A}').contains(&chars[i + 1]) {
            i += 3;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    for p in ["ถนน", "ซอย", "หมู่"] {
        out = out.replace(p, &format!(" {} ", p));
    }
    for p in ["ตำบล", "อำเภอ", "จังหวัด", "แขวง", "เขต"] {
        out = out.replace(p, " ");
    }
    let words: Vec<&str> = out.split(' ').filter(|w| !matches!(w.to_lowercase().as_str(), "subdistrict" | "district" | "province" | "tambon" | "amphoe" | "changwat")).collect();
    words.join(" ")
}

/// Korean road names carry their own numbers and type words: "예제대로1418번길", "Yeje-daero
/// 1418beon-gil", "Yeje 2-ro 22beon-gil". Join each into one romanized word, written the same way
/// in both scripts, so the numbers are not read as house numbers and the spelling of the joins
/// does not matter.
fn merge_korean_roads(toks: Vec<crate::token::Tok>) -> Vec<crate::token::Tok> {
    let is_type = |s: &str| matches!(s, "로" | "길" | "번길" | "대로" | "가길" | "ro" | "gil" | "beongil" | "daero" | "dae" | "lo" | "beon" | "ga" | "gagil" | "번" | "가");
    let hangul = |c: char| ('\u{AC00}'..='\u{D7AF}').contains(&c);
    let numeric = |s: &str| s.chars().next().map_or(false, |c| c.is_ascii_digit()) && s.trim_start_matches(|c: char| c.is_ascii_digit()).chars().all(|c| c.is_ascii_alphabetic() || hangul(c));
    let road_end = |s: &str| is_type(s) || (s.chars().count() >= 2 && (s.ends_with('로') || s.ends_with('길')) && s.chars().all(hangul));
    // (token, original segment)
    let mut out: Vec<(crate::token::Tok, usize)> = Vec::with_capacity(toks.len());
    let mut shift = 0;
    let mut i = 0;
    while i < toks.len() {
        let mut t = toks[i].clone();
        let seg = t.seg;
        t.seg += shift;
        out.push((t, seg));
        i += 1;
        // a run ends at a type word that is followed by no further road part
        let last = &out.last().unwrap().0;
        let next_continues = i < toks.len() && toks[i].seg == seg && (is_type(&toks[i].s) || (numeric(&toks[i].s) && toks.get(i + 1).map_or(false, |t| t.seg == seg && is_type(&t.s))));
        if !road_end(&last.s) || next_continues {
            continue;
        }
        // walk back over numbers and type words to the name word
        let mut k = out.len() - 1;
        let mut named = !is_type(&out[k].0.s);
        while !named && k > 0 && out[k - 1].1 == seg && !out[k - 1].0.abbr {
            k -= 1;
            let s = &out[k].0.s;
            if is_type(s) || numeric(s) {
                continue;
            }
            if s.chars().all(|c| c.is_alphabetic()) {
                named = true;
            } else {
                break;
            }
        }
        if !named || (k == out.len() - 1 && !(k > 0 && out[k - 1].1 == seg && out[k].0.s.chars().all(hangul))) {
            continue;
        }
        // one spelling for both scripts: final consonants as RR writes them, r and l alike
        let joined: String = out[k..].iter().map(|t| crate::fold::ascii_key(&t.0.s)).collect::<String>().replace('r', "l").replace('k', "g").replace('t', "d").replace('p', "b").replace("nl", "ll").replace("ln", "ll");
        let first = out[k].0.clone();
        // words before the road in the same segment are places ("통영시 무전3길 23-7"): the road
        // starts a segment of its own, and so does what follows
        let split = k > 0 && out[k - 1].1 == seg;
        out.truncate(k);
        let mut tok = crate::token::Tok { s: joined, script: Script::Latin, num: false, abbr: false, ..first };
        if split {
            shift += 1;
            tok.seg += 1;
        }
        out.push((tok, seg));
    }
    out.into_iter().map(|x| x.0).collect()
}

pub fn parse(ctx: &Ctx, text: &str) -> Components {
    let text = drop_paren_articles(text);
    let text = join_dotted_compass(text.as_ref()).into_owned();
    let text = text.as_str();
    let mut folded = ctx.cjk.apply(&fold(text));
    if ctx.cc == Some("KR") {
        folded = ctx.cjk.korean_romanized(&folded);
    }
    if ctx.cc == Some("TH") || folded.chars().any(|ch| ('\u{0E00}'..='\u{0E7F}').contains(&ch)) {
        folded = thai_fold(&folded);
    }
    // Ukrainian and Belarusian Г is romanized "h" ("Гора" / "Hora")
    if matches!(ctx.cc, Some("UA") | Some("BY")) && folded.contains('г') {
        folded = folded.replace('г', "h");
    }
    // Korean city ranks written onto the name ("서울특별시", "인천광역시") are left out in
    // romanizations ("Seoul", "Incheon")
    if folded.contains("특별") || folded.contains("광역시") {
        for w in ["특별자치시", "특별자치도", "특별시", "광역시"] {
            folded = folded.replace(w, "");
        }
    }
    let jp = ctx.cc == Some("JP") || folded.chars().any(|ch| script_of(ch) == Script::Kana);
    let mut toks = tokenize(&folded, jp);
    // Korean road names carry numbers: "효목로5길 55", "Seoyang-ro 49beon-gil 42". The number before
    // "gil" / "beongil" names the road; it is not the house number
    if ctx.cc == Some("KR") {
        for q in 0..toks.len().saturating_sub(1) {
            if toks[q].num && toks[q + 1].seg == toks[q].seg && matches!(toks[q + 1].s.as_str(), "gil" | "beongil" | "beon" | "ga" | "gagil" | "길" | "번길") {
                toks[q].num = false;
            }
        }
    }
    if ctx.cc == Some("KR") || folded.chars().any(|ch| script_of(ch) == Script::Hangul) {
        toks = merge_korean_roads(toks);
    }
    let script = Some(dominant_script(&folded));
    let n = toks.len();
    let mut nseg = toks.last().map_or(0, |t| t.seg + 1);
    // dictionary spans (longest match within a segment)
    let mut spans: Vec<Option<Span>> = vec![None; n];
    let mut i = 0;
    while i < n {
        let mut found = None;
        for len in (1..=ctx.data.max_phrase.min(n - i)).rev() {
            if toks[i + len - 1].seg != toks[i].seg {
                continue;
            }
            if len == 1 && toks[i].num {
                break;
            }
            // phrases made only of single letters ("c s") are initials, not dictionary phrases
            if len >= 2 && toks[i..i + len].iter().all(|t| t.s.chars().count() == 1) {
                continue;
            }
            let key: Vec<&str> = toks[i..i + len].iter().map(|t| t.s.as_str()).collect();
            let es = lookup(ctx, &key.join(" "));
            if !es.is_empty() {
                found = Some(Span { start: i, len, entries: es });
                break;
            }
        }
        match found {
            Some(sp) => {
                let l = sp.len;
                for k in i..i + l {
                    spans[k] = Some(sp.clone());
                }
                i += l;
            }
            None => i += 1,
        }
    }
    let span_at = |k: usize| -> Option<&Span> { spans[k].as_ref() };
    let is_start = |k: usize| spans[k].as_ref().map_or(false, |s| s.start == k);

    let mut used = vec![false; n];
    let parts = toks.iter().map(|t| t.seg).collect::<std::collections::BTreeSet<_>>().len();
    let mut c = Components { script, cc: ctx.cc.map(|x| x.to_string()), cc_inferred: ctx.inferred, parts, ..Default::default() };
    let mut house_kw_next: Vec<usize> = Vec::new();

    // value token after a keyword span starting at k (skip "#", "no", ":" style tokens)
    let value_after = |k: usize, used: &Vec<bool>| -> Option<usize> {
        let sp = span_at(k)?;
        let mut j = sp.start + sp.len;
        let mut hops = 0;
        while j < n && toks[j].seg == toks[k].seg && hops < 2 {
            if used[j] {
                return None;
            }
            if toks[j].num {
                return Some(j);
            }
            if toks[j].s == "#" || span_at(j).map_or(false, |s| s.has(Class::HouseKw)) {
                j += 1;
                hops += 1;
                continue;
            }
            let t = &toks[j].s;
            let tl = t.chars().count();
            // letters glued to digits in the original ("bt03", "H0203")
            if tl <= 2 && t.chars().all(|ch| ch.is_alphabetic()) && j + 1 < n && toks[j + 1].seg == toks[j].seg && toks[j + 1].num {
                return Some(j);
            }
            if tl <= 2 && t.chars().all(|ch| ch.is_alphanumeric()) && (tl == 1 || !span_at(j).map_or(false, |s| s.has(Class::Stop) || s.has(Class::StreetType))) {
                return Some(j);
            }
            // a short code closing the segment ("Ap. DTO", "Flat GF", "Apt PH")
            let last_in_seg = j + 1 >= n || toks[j + 1].seg != toks[j].seg;
            if last_in_seg && tl <= 4 && t.chars().all(|ch| ch.is_alphanumeric()) {
                return Some(j);
            }
            // unit codes such as "a2", "bt05", "h0103"
            if t.len() <= 6 && t.chars().all(|ch| ch.is_ascii_alphanumeric()) && t.chars().any(|ch| ch.is_ascii_digit()) {
                return Some(j);
            }
            return None;
        }
        None
    };

    // 1. unit, level, PO box, house-number keywords
    for k in 0..n {
        if !is_start(k) || used[k] {
            continue;
        }
        let sp = span_at(k).unwrap().clone();
        let po = sp.has(Class::PoBox);
        let unit = sp.has(Class::UnitNum) || sp.has(Class::LevelNum) || sp.has(Class::Staircase) || sp.has(Class::Entrance);
        let hkw = sp.has(Class::HouseKw);
        if po && !unit {
            if let Some(v) = value_after(k, &used).filter(|&v| toks[v].s.chars().any(|ch| ch.is_ascii_digit()) || (v + 1 < n && toks[v + 1].num)) {
                c.po_box = Some(norm_number(&toks[v].s));
                for x in k..=v {
                    used[x] = true;
                }
                continue;
            }
        }
        let kw_len: usize = toks[k..k + sp.len].iter().map(|t| t.s.chars().count()).sum();
        let region_kw = ctx.cc.and_then(|cc| ctx.data.countries.get(cc)).map_or(false, |co| co.regions.contains_key(&ascii_key(&toks[k].s)));
        let seg_k = toks[k].seg;
        let seg_free = (0..n).filter(|&q| toks[q].seg == seg_k && !used[q]).count();
        let seg_typed = (0..n).any(|q| toks[q].seg == seg_k && q != k && span_at(q).map_or(false, |s| s.has(Class::StreetType) && s.start != sp.start));
        // "Noordkade 104 bus B Waddinxveen", "104 Noordkade bus B": a unit word after the house
        // number's word group, even without commas
        let after_num = (0..k).any(|q| toks[q].seg == seg_k && toks[q].num && !used[q]);
        let unit_ctx = seg_free <= sp.len + 2 || seg_typed || after_num || ctx.cjk_like(script);
        let pure_unit = seg_free == sp.len + 1 && value_after(k, &used).map_or(false, |v| toks[v].num);
        // "D15 L4G7", "P1T 9F7": letters that start a postcode are not unit keywords
        let zip_here = ctx.cc.and_then(|cc| ctx.data.countries.get(cc)).and_then(|co| co.zip()).map_or(false, |re| {
            (2..=4).any(|l| k + l <= n && toks[k + l - 1].seg == toks[k].seg && re.is_match(&toks[k..k + l].iter().map(|t| t.s.as_str()).collect::<String>()))
        });
        let short_unit_kw = kw_len <= 1 && !toks[k + sp.len - 1].abbr;
        let value_is_zip = (region_kw || kw_len <= 2)
            && value_after(k, &used).map_or(false, |v| ctx.cc.and_then(|cc| ctx.data.countries.get(cc)).and_then(|co| co.zip()).map_or(false, |re| re.is_match(&toks[v].s)));
        // with no country: a state or province code of a country that writes one before its
        // postcode, before a postcode of that country's form ("FL 32500"), is no unit
        let value_is_zip = value_is_zip
            || (ctx.cc.is_none()
                && value_after(k, &used).map_or(false, |v| {
                    let kw: String = toks[k..k + sp.len].iter().map(|t| t.s.as_str()).collect();
                    ["US", "CA", "AU"].iter().filter_map(|cc| ctx.data.countries.get(*cc)).any(|co| co.regions.contains_key(&crate::fold::ascii_key(&kw)) && co.zip().map_or(false, |re| re.is_match(&toks[v].s)))
                }));
        if unit && !sp.has(Class::StreetType) && (!region_kw || pure_unit) && !value_is_zip && unit_ctx && !zip_here && !short_unit_kw {
            if let Some(v) = value_after(k, &used).filter(|&v| toks[v].s.chars().any(|ch| ch.is_ascii_digit()) || kw_len >= 3 || toks[k + sp.len - 1].abbr) {
                let kind = if sp.has(Class::LevelNum) { "l" } else { "u" };
                // "H0101" and "A2" are split into letters and digits by the tokenizer: rejoin them
                let mut end = v;
                let mut val = norm_number(&toks[v].s);
                if !toks[v].num && toks[v].s.chars().count() <= 2 && v + 1 < n && toks[v + 1].seg == toks[v].seg && toks[v + 1].num && !used[v + 1] {
                    val.push_str(&norm_number(&toks[v + 1].s));
                    end = v + 1;
                }
                c.units.push(format!("{}{}", kind, val));
                c.unit_words.push(toks[k..=end].iter().map(|t| t.s.as_str()).collect::<Vec<_>>().join(" "));
                for x in k..=end {
                    used[x] = true;
                }
                continue;
            }
        }
        if unit && sp.has(Class::StreetType) && !value_is_zip {
            // ambiguous words like "pl" or "ste": treat as unit only when a number follows directly
            if let Some(v) = value_after(k, &used) {
                let glued = !toks[v].num && toks[v].s.chars().count() <= 2 && v + 1 < n && toks[v + 1].seg == toks[v].seg && toks[v + 1].num && !used[v + 1];
                let pure = seg_free == sp.len + 1 || (glued && seg_free == sp.len + 2);
                if v == sp.start + sp.len && (toks[v].num || glued || (pure && toks[v].s.chars().count() <= 2 && toks[k + sp.len - 1].abbr)) && (pure || (k > 0 && toks[k - 1].seg == toks[k].seg && (0..k).any(|q| toks[q].num && toks[q].seg == toks[k].seg))) {
                    let kind = if sp.has(Class::LevelNum) { "l" } else { "u" };
                    let mut val = norm_number(&toks[v].s);
                    let mut end = v;
                    if glued {
                        val.push_str(&norm_number(&toks[v + 1].s));
                        end = v + 1;
                    }
                    c.units.push(format!("{}{}", kind, val));
                    c.unit_words.push(toks[k..=end].iter().map(|t| t.s.as_str()).collect::<Vec<_>>().join(" "));
                    for x in k..=end {
                        used[x] = true;
                    }
                    continue;
                }
            }
        }
        let short_kw = kw_len <= 1 && !toks[k + sp.len - 1].abbr;
        if hkw && !unit && !short_kw {
            if let Some(v) = value_after(k, &used) {
                if toks[v].num && (!sp.has(Class::StreetType) || v == sp.start + sp.len) {
                    house_kw_next.push(v);
                    for x in k..k + sp.len {
                        used[x] = true;
                    }
                }
            }
        }
    }
    // "#" followed by a value: unit when a number was seen before it in the segment, else house kw
    for k in 0..n {
        if toks[k].s != "#" || used[k] {
            continue;
        }
        let nxt = k + 1;
        if nxt < n && toks[nxt].seg == toks[k].seg && !used[nxt] && toks[nxt].s.chars().any(|ch| ch.is_alphanumeric()) {
            let prior_num = (0..k).any(|q| toks[q].num && !used[q] && toks[q].seg <= toks[k].seg);
            if prior_num {
                c.units.push(format!("u{}", norm_number(&toks[nxt].s)));
                c.unit_words.push(format!("{} {}", toks[k].s, toks[nxt].s));
                used[k] = true;
                used[nxt] = true;
            } else if toks[nxt].num {
                house_kw_next.push(nxt);
                used[k] = true;
            }
        } else {
            used[k] = true;
        }
    }
    // number followed by a level word ("3e étage", "2. OG")
    for k in 0..n.saturating_sub(1) {
        if toks[k].num && !used[k] && toks[k + 1].seg == toks[k].seg && is_start(k + 1) {
            let sp = span_at(k + 1).unwrap();
            let wl: usize = toks[k + 1..k + 1 + sp.len].iter().map(|t| t.s.chars().count()).sum();
            let segk = toks[k].seg;
            let free = (0..n).filter(|&q| toks[q].seg == segk && !used[q]).count();
            let typed = (0..n).any(|q| toks[q].seg == segk && spans[q].as_ref().map_or(false, |s| s.has(Class::StreetType)));
            if (sp.has(Class::LevelNum) || sp.has(Class::LevelAlone)) && !sp.has(Class::StreetType) && (wl >= 2 || toks[k + 1].abbr) && (free <= sp.len + 1 || typed) {
                c.units.push(format!("l{}", norm_number(&toks[k].s)));
                c.unit_words.push(toks[k..k + 1 + sp.len].iter().map(|t| t.s.as_str()).collect::<Vec<_>>().join(" "));
                for x in k..k + 1 + sp.len {
                    used[x] = true;
                }
            }
        }
    }
    // standalone unit/level words without number (penthouse, ground floor): record as unit
    for k in 0..n {
        if is_start(k) && !used[k] {
            let sp = span_at(k).unwrap();
            if (sp.has(Class::LevelAlone) || sp.has(Class::UnitAlone)) && !sp.has(Class::StreetType) && !sp.has(Class::Synonym) {
                // only when the segment has nothing else (a unit segment)
                let seg = toks[k].seg;
                let others = (0..n).any(|q| toks[q].seg == seg && !(sp.start..sp.start + sp.len).contains(&q) && !used[q]);
                if !others {
                    let ck = &ctx.data.canon_keys[sp.entries[0].canon as usize];
                    c.units.push(format!("s{}", ck));
                    c.unit_words.push(toks[sp.start..sp.start + sp.len].iter().map(|t| t.s.as_str()).collect::<Vec<_>>().join(" "));
                    for x in sp.start..sp.start + sp.len {
                        used[x] = true;
                    }
                }
            }
        }
    }

    // street-type positions per segment
    let mut seg_has_type = vec![false; nseg];
    let mut seg_type_pos = vec![usize::MAX; nseg];
    for k in 0..n {
        if !used[k] && span_at(k).map_or(false, |s| s.has(Class::StreetType)) {
            let s = toks[k].seg;
            seg_has_type[s] = true;
            if seg_type_pos[s] == usize::MAX {
                seg_type_pos[s] = k;
            }
        }
    }

    // 2. postcode
    let first_word_seg = (0..n).find(|&q| !toks[q].num && !used[q] && toks[q].s != "#").map(|q| toks[q].seg);
    let first_numword_seg = (0..nseg).find(|&sg| {
        (0..n).any(|q| toks[q].seg == sg && toks[q].num && !used[q]) && (0..n).any(|q| toks[q].seg == sg && !toks[q].num && !used[q] && toks[q].s != "#")
    });
    let zip = ctx.cc.and_then(|cc| ctx.data.countries.get(cc)).and_then(|c| c.zip());
    let mut postcode_span: Option<(usize, usize)> = None;
    'outer: for k in 0..n {
        if used[k] || house_kw_next.contains(&k) {
            continue;
        }
        for len in (1..=4usize).rev() {
            if k + len > n || toks[k + len - 1].seg != toks[k].seg || (k..k + len).any(|x| used[x]) {
                continue;
            }
            if !toks[k..k + len].iter().any(|t| t.s.chars().any(|ch| ch.is_ascii_digit())) {
                continue;
            }
            if !toks[k..k + len].iter().all(|t| t.s.is_ascii()) {
                continue;
            }
            // cheap shape filter before the (slow) regex: postcodes are short and start with a
            // digit or a short letter group
            let total: usize = toks[k..k + len].iter().map(|t| t.s.len()).sum();
            if total < 3 || total > 10 || (!toks[k].num && toks[k].s.len() > 4) || toks[k..k + len].iter().any(|t| t.s.len() > 8) {
                continue;
            }
            let joined: String = toks[k..k + len].iter().map(|t| t.s.as_str()).collect::<Vec<_>>().join(" ");
            let ok = match zip {
                Some(re) => re.is_match(&joined) || (len > 1 && re.is_match(&joined.replace(' ', ""))),
                None => len == 1 && toks[k].num && (4..=6).contains(&toks[k].s.len()) && toks[k].s.chars().all(|c| c.is_ascii_digit()),
            };
            if !ok {
                continue;
            }
            // guard: the first number of the first segment holding numbers and words is the house number
            let seg = toks[k].seg;
            let first_num = (0..n).find(|&q| toks[q].seg == seg && toks[q].num && !used[q]);
            let typed_before = (0..seg).any(|s2| seg_has_type[s2] && !(0..n).any(|q| toks[q].seg == s2 && toks[q].num && !used[q]));
            let bare_before = (0..seg).any(|s2| {
                let ks: Vec<usize> = (0..n).filter(|&q| toks[q].seg == s2 && !used[q]).collect();
                !ks.is_empty() && ks.iter().all(|&q| toks[q].num)
            });
            let _ = first_word_seg;
            // "1016 Budapest, Dezső utca 11.": a later typed segment holds the house number
            let typed_after = !seg_has_type[seg] && (seg + 1..nseg).any(|s2| seg_has_type[s2] && (0..n).any(|q| toks[q].seg == s2 && toks[q].num && !used[q]));
            // a postcode right after a state or province code ends its part ("Exampleton, IL 60000"),
            // where states are written before the postcode
            // (a part of its own, the state and postcode alone, may open the text too: "OH 46010,
            // Exampleton", the area after a street)
            let alone = nseg > 1 && (0..n).filter(|&q| toks[q].seg == seg && !used[q]).count() == len + 1;
            let after_state = (seg > 0 || alone)
                && k > 0
                && toks[k - 1].seg == seg
                && k + len == n.min((k..n).find(|&q| toks[q].seg != seg).unwrap_or(n))
                && {
                    let code = toks[k - 1].s.as_str();
                    let states = |cc: &str| ctx.data.countries.get(cc).map_or(false, |co| co.regions.contains_key(code));
                    code.len() <= 3 && match ctx.cc {
                        None => ["US", "CA", "AU"].iter().any(|cc| states(cc)),
                        Some(cc) => matches!(cc, "US" | "CA" | "AU") && states(cc),
                    }
                };
            if Some(seg) == first_numword_seg && first_num == Some(k) && !typed_before && !bare_before && !typed_after && !ctx.cjk_like(script) && !after_state {
                continue;
            }
            let seg_only_this = (0..n).filter(|&q| toks[q].seg == seg && !used[q]).count() == len;
            let typed_adjacent = (seg > 0 && seg_has_type[seg - 1]) || (seg + 1 < nseg && seg_has_type[seg + 1]);
            if seg_only_this && typed_adjacent && (0..n).all(|q| !(toks[q].num && !used[q] && toks[q].seg < seg && q != k)) {
                continue;
            }
            if zip.is_none() && seg == 0 {
                continue;
            }
            postcode_span = Some((k, len));
            break 'outer;
        }
    }
    if let Some((k, len)) = postcode_span {
        let joined: String = toks[k..k + len].iter().map(|t| t.s.as_str()).collect::<String>();
        let mut pc: String = joined.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase();
        if ctx.cc == Some("US") && pc.len() > 5 {
            pc.truncate(5);
        }
        c.postcode = Some(pc);
        for x in k..k + len {
            used[x] = true;
        }
        if k > 0 && toks[k - 1].seg == toks[k].seg && !used[k - 1] {
            let p = toks[k - 1].s.as_str();
            let own = ctx.cc.map_or(false, |cc| cc.eq_ignore_ascii_case(p));
            // a country prefix ("D-10115", "FL-9490"), unless it is a region of the hinted country
            // ("FL 33100" in the US)
            let region = ctx.cc.and_then(|cc| ctx.data.countries.get(cc)).map_or(false, |co| co.regions.contains_key(p));
            if own || (!region && matches!(p, "d" | "f" | "a" | "ch" | "l" | "b" | "i" | "nl" | "lt" | "lv" | "ee" | "fi" | "s" | "dk" | "n" | "pl" | "cz" | "sk" | "e" | "p" | "fl")) {
                used[k - 1] = true;
            }
        }
    }

    // 3. house number and street segment
    let mut house_idx: Option<usize> = None;
    for &v in &house_kw_next {
        if !used[v] {
            house_idx = Some(v);
            break;
        }
    }
    let street_seg: Option<usize>;
    let type_segs: Vec<usize> = (0..nseg).filter(|&s| seg_has_type[s]).collect();
    if house_idx.is_none() {
        // left to right: the first segment with a number and words, or a typed street segment
        // followed (or preceded) by a bare number segment
        'scan: for s in 0..nseg {
            let nums: Vec<usize> = (0..n).filter(|&q| toks[q].seg == s && toks[q].num && !used[q]).collect();
            // "135 ter" alone is a number with its suffix, not a street with a number
            let is_suffix = |q: usize| q > 0 && toks[q - 1].num && toks[q - 1].seg == toks[q].seg && (matches!(toks[q].s.as_str(), "bis" | "ter" | "quater") || (toks[q].s.chars().count() == 1 && toks[q].s.chars().all(|ch| ch.is_ascii() || matches!(script_of(ch), Script::Cyrillic | Script::Greek))));
            let words = (0..n).any(|q| toks[q].seg == s && !toks[q].num && !used[q] && toks[q].s != "#" && !is_suffix(q));
            if !nums.is_empty() && words && seg_has_type[s] && s + 1 < nseg {
                // "str. 31 August 1989, 121a": a lone number segment after a typed street holds the
                // house number; the street's own numbers are part of its name
                let ks: Vec<usize> = (0..n).filter(|&q| toks[q].seg == s + 1 && !used[q]).collect();
                if !ks.is_empty() && toks[ks[0]].num && (ks.len() == 1 || (ks.len() == 2 && is_suffix(ks[1]))) {
                    house_idx = Some(ks[0]);
                    break 'scan;
                }
            }
            if !nums.is_empty() && words {
                // "бульвар 40 лет Октября 24", "Avenida 9 de Julio 1234": a type before the first
                // number and words after it make the last number the house number
                let first = nums[0];
                let type_before = (0..first).any(|q| toks[q].seg == s && !used[q] && spans[q].as_ref().map_or(false, |sp| sp.has(Class::StreetType)));
                let word_after = first + 1 < n && toks[first + 1].seg == s && !toks[first + 1].num && !used[first + 1] && toks[first + 1].s.chars().count() > 1;
                let number_first_cc = matches!(ctx.cc, Some("US" | "CA" | "GB" | "IE" | "AU" | "NZ" | "ZA" | "SG" | "IN" | "FR" | "BE" | "LU" | "MY" | "PH" | "VN" | "TH" | "HK" | "AE" | "SA" | "EG" | "QA" | "KW" | "BH" | "OM"));
                let type_then_num = first > 0 && toks[first - 1].seg == s && !used[first - 1] && spans[first - 1].as_ref().map_or(false, |sp| sp.has(Class::StreetType));
                let last = *nums.last().unwrap();
                let last_is_tail = (last + 1..n).all(|q| toks[q].seg != s || used[q] || (toks[q].s.chars().count() == 1 && toks[q].s.is_ascii()));
                let _ = type_then_num;
                let seg_start = (0..n).find(|&q| toks[q].seg == s && !used[q]);
                let words_before_last = (0..last).any(|q| toks[q].seg == s && !used[q] && !toks[q].num);
                if number_first_cc && seg_start == Some(first) {
                    house_idx = Some(first);
                } else if nums.len() >= 2 && last_is_tail && words_before_last {
                    // "Pasaje 12 1694", "Vanaginės 3-oji g. 17", "17. listopadu 258": the number closing
                    // the segment is the house number; earlier numbers belong to the street name
                    house_idx = Some(last);
                } else if nums.len() >= 2 && type_before && word_after {
                    house_idx = Some(last);
                } else {
                    house_idx = Some(first);
                }
                break;
            }
            if seg_has_type[s] && nums.is_empty() {
                for adj in [s + 1, s.wrapping_sub(1)] {
                    if adj < nseg {
                        let ks: Vec<usize> = (0..n).filter(|&q| toks[q].seg == adj && !used[q]).collect();
                        if ks.len() == 1 && toks[ks[0]].num {
                            house_idx = Some(ks[0]);
                            break 'scan;
                        }
                        // "Paseo Guzman, 6 R7049CPJ Rosario": the number leads the next segment
                        let number_first_cc = matches!(ctx.cc, Some("US" | "CA" | "GB" | "IE" | "AU" | "NZ" | "ZA" | "SG" | "IN" | "FR" | "BE" | "LU" | "MY" | "PH" | "VN" | "TH" | "HK" | "AE" | "SA" | "EG" | "QA" | "KW" | "BH" | "OM"));
                        if adj == s + 1 && ks.len() > 1 && toks[ks[0]].num && !number_first_cc {
                            let h = ks[0];
                            let skip = if ks.len() > 1 && is_suffix(ks[1]) { 2 } else { 1 };
                            if ks.len() > skip {
                                let new_seg = nseg;
                                nseg += 1;
                                seg_has_type.push(false);
                                for &q in &ks[skip..] {
                                    toks[q].seg = new_seg;
                                }
                            }
                            house_idx = Some(h);
                            break 'scan;
                        }
                    }
                }
            }
        }
    }
    if house_idx.is_none() {
        house_idx = (0..n).find(|&q| toks[q].num && !used[q] && (0..n).any(|w| toks[w].seg == toks[q].seg && !toks[w].num && !used[w]));
    }
    if house_idx.is_none() {
        house_idx = (0..n).find(|&q| toks[q].num && !used[q]);
    }
    if let Some(h) = house_idx {
        let mut hn = norm_number(&toks[h].s);
        used[h] = true;
        // AU/NZ "3/12": unit 3, number 12
        if hn.contains('/') {
            let (a, b) = hn.split_once('/').unwrap();
            let (a, b) = (a.to_string(), b.to_string());
            if b.len() == 1 && b.chars().all(|ch| ch.is_ascii_alphabetic()) {
                // "11/B": a letter after the slash is the number's own suffix
                hn = format!("{}{}", a, b);
            } else if matches!(ctx.cc, Some("AU") | Some("NZ")) {
                c.units.push(format!("u{}", a));
                hn = b;
            } else if !matches!(ctx.cc, Some("CZ") | Some("SK")) {
                c.units.push(format!("u{}", b));
                hn = a;
            }
        }
        // suffix letter or bis/ter
        let nxt = h + 1;
        if nxt < n && toks[nxt].seg == toks[h].seg && !used[nxt] {
            let t = &toks[nxt].s;
            let is_bis = matches!(t.as_str(), "bis" | "ter" | "quater" | "b" | "a" | "c" | "d");
            // any script's letter ("8-А", "95б"), written as its ASCII key
            let single = t.chars().count() == 1 && t.chars().all(|ch| ch.is_alphabetic() && (ch.is_ascii() || matches!(script_of(ch), Script::Cyrillic | Script::Greek)));
            let last_in_seg = nxt + 1 >= n || toks[nxt + 1].seg != toks[nxt].seg;
            let next_is_dirlike = span_at(nxt).map_or(false, |s| s.has(Class::Directional)) && !last_in_seg;
            let number_first_here = matches!(ctx.cc, Some("US" | "CA" | "GB" | "IE" | "AU" | "NZ" | "ZA" | "SG" | "IN" | "PH" | "MY"));
            let lead = (0..h).all(|q| toks[q].seg != toks[h].seg || used[q]);
            // where the number comes first, a lone Latin letter between it and a street of two words
            // or more is the number's suffix ("12 A Example St" is "12A Example St"; in "12 A St" and
            // "12 N Example St" the letter is the street's)
            let typed = |q: usize| span_at(q).map_or(false, |s| s.has(Class::StreetType) || s.has(Class::Directional));
            let spaced = matches!(ctx.cc, Some("US" | "CA" | "GB" | "IE" | "AU" | "NZ"))
                && text.contains(&format!("{} {} ", toks[h].s, t.to_uppercase()))
                && lead
                && single
                && t.is_ascii()
                && !typed(nxt)
                && nxt + 2 < n
                && toks[nxt + 1].seg == toks[nxt].seg
                && toks[nxt + 2].seg == toks[nxt].seg
                && !toks[nxt + 1].num
                && !typed(nxt + 1);
            if spaced || (is_bis && matches!(t.as_str(), "bis" | "ter" | "quater")) || (single && last_in_seg) || (single && lead && !number_first_here && !next_is_dirlike && !toks[nxt].abbr && !span_at(nxt).map_or(false, |s| s.has(Class::StreetType) || s.has(Class::Title) || s.has(Class::Synonym))) || (single && !next_is_dirlike && nxt + 1 < n && span_at(nxt + 1).map_or(false, |s| s.has(Class::UnitNum))) {
                hn.push_str(&crate::fold::ascii_key(t));
                used[nxt] = true;
            }
        }
        c.house = Some(hn.to_ascii_lowercase());
        // a part of two numbers and no words ("30 18, Exampleton"): the second is the place's own
        // number too, of a numbered street or a building, kept apart from the house number
        let part: Vec<usize> = (0..n).filter(|&q| toks[q].seg == toks[h].seg).collect();
        if !ctx.cjk_like(script) && part.len() == 2 && part[0] == h && toks[part[1]].num && !used[part[1]] && toks[part[1]].s.chars().all(|ch| ch.is_ascii_digit()) && toks[part[1]].s.len() <= 4 {
            c.second_number = Some(norm_number(&toks[part[1]].s));
            used[part[1]] = true;
        }
        // CJK and Hangul: a bare number after the block number is a room ("1-2-3 405")
        if ctx.cjk_like(script) {
            for q in h + 1..n {
                if toks[q].seg == toks[h].seg && toks[q].num && !used[q] {
                    c.units.push(format!("u{}", norm_number(&toks[q].s)));
                    used[q] = true;
                }
            }
        }
    }
    // 3b. no commas: split the house-number segment into business / street / locality parts
    if let Some(h) = house_idx {
        let sg = toks[h].seg;
        let is_type = |q: usize| spans[q].as_ref().map_or(false, |s| s.has(Class::StreetType) && !s.has(Class::HouseKw));
        let free: Vec<usize> = (0..n).filter(|&q| toks[q].seg == sg && !used[q] && toks[q].s != "#").collect();
        let before: Vec<usize> = free.iter().copied().filter(|&q| q < h).collect();
        let after: Vec<usize> = free.iter().copied().filter(|&q| q > h).collect();
        let number_first = matches!(ctx.cc, Some("US" | "CA" | "GB" | "IE" | "AU" | "NZ" | "ZA" | "SG" | "IN" | "FR" | "BE" | "LU" | "MY" | "PH" | "VN" | "TH" | "HK" | "AE" | "SA" | "EG" | "QA" | "KW" | "BH" | "OM"));
        let postcode_after = postcode_span.map(|(k, _)| k).filter(|&k| toks[k].seg == sg && k > h);
        // (from, to) token ranges that become their own segments
        let mut tail_from: Option<usize> = None;
        let mut head_to: Option<usize> = None; // business words before this index
        if !after.is_empty() && !ctx.cjk_like(script) {
            if before.is_empty() || (number_first && after.iter().any(|&q| is_type(q))) {
                if !before.is_empty() {
                    head_to = Some(h);
                }
                // number first: street runs to the first type after a name word, or up to the postcode
                let mut end: Option<usize> = None;
                let later_segs = (0..n).any(|q| toks[q].seg > sg && !used[q]);
                if !later_segs || postcode_after.is_some() {
                    for &q in after.iter() {
                        if is_type(q) && spans[q].as_ref().map_or(false, |sp| sp.start == q && sp.start > after[0] && !sp.has(Class::Stop)) {
                            let sp = spans[q].as_ref().unwrap();
                            end = Some(sp.start + sp.len - 1);
                        }
                    }
                    // the street must keep a word after the last type only if that word is a type too
                    if let Some(e) = end {
                        if !after.iter().any(|&q| q > e) {
                            end = None;
                        }
                    }
                }
                if end.is_none() {
                    if let Some(pk) = postcode_after {
                        end = after.iter().copied().filter(|&q| q < pk).last();
                    }
                }
                if let Some(e) = end {
                    tail_from = after.iter().copied().find(|&q| q > e);
                }
            } else {
                // street before number: business words may precede a leading street type
                let tpos = before.iter().position(|&q| is_type(q) && spans[q].as_ref().unwrap().start == q);
                if let Some(tp) = tpos {
                    if tp >= 1 && tp + 1 < before.len() {
                        head_to = Some(before[tp]);
                    }
                }
                // a letter suffix right after the number stays with it
                let first_after = after[0];
                let skip = if first_after == h + 1 && toks[first_after].s.chars().count() == 1 && toks[first_after].s.is_ascii() { 1 } else { 0 };
                tail_from = after.get(skip).copied();
            }
        }
        if let Some(t) = tail_from {
            let new_seg = nseg;
            nseg += 1;
            for q in t..n {
                if toks[q].seg == sg {
                    toks[q].seg = new_seg;
                }
            }
            seg_has_type.push(false);
        }
        if let Some(b) = head_to {
            let new_seg = nseg;
            nseg += 1;
            for q in 0..b {
                if toks[q].seg == sg && !used[q] {
                    toks[q].seg = new_seg;
                }
            }
            seg_has_type.push(false);
        }
        // keep segment ids ordered by position for the later before/after logic (positions are used)
        let _ = nseg;
    }
    let toks = toks;

    // street segment
    street_seg = if let Some(h) = house_idx {
        let s = toks[h].seg;
        let words = (0..n).any(|q| toks[q].seg == s && !used[q] && !toks[q].num && toks[q].s != "#");
        if words {
            Some(s)
        } else if let Some(&ts) = type_segs.iter().min_by_key(|&&ts| (ts as isize - s as isize).abs()) {
            Some(ts)
        } else if s > 0 && (0..n).any(|q| toks[q].seg == s - 1 && !used[q]) {
            Some(s - 1)
        } else if s + 1 < nseg {
            Some(s + 1)
        } else {
            None
        }
    } else {
        type_segs.first().copied()
    };

    // 4. street name and type
    let mut street_lo = usize::MAX;
    let mut street_hi = 0usize;
    if let Some(ss) = street_seg {
        let mut idx: Vec<usize> = (0..n).filter(|&q| toks[q].seg == ss && !used[q] && !toks[q].num && toks[q].s != "#").collect();
        if !ctx.cjk_like(script) && !idx.is_empty() {
            let number_first_cc = matches!(ctx.cc, Some("US" | "CA" | "GB" | "IE" | "AU" | "NZ" | "ZA" | "SG" | "IN" | "FR" | "BE" | "LU" | "MY" | "PH" | "VN" | "TH" | "HK" | "AE" | "SA" | "EG" | "QA" | "KW" | "BH" | "OM"));
            let h_same = house_idx.filter(|&h| toks[h].seg == ss);
            let (lo, hi) = (idx[0], *idx.last().unwrap());
            let extra: Vec<usize> = (0..n)
                .filter(|&q| toks[q].seg == ss && !used[q] && toks[q].num)
                .filter(|&q| match h_same {
                    // inside the name, or on the far side of the name from the house number
                    Some(h) => (q > lo && q < hi) || (h < lo && q > h && (q < lo || number_first_cc)) || (h > hi && q < h),
                    None => true,
                })
                .collect();
            if !extra.is_empty() {
                idx.extend(extra);
                idx.sort();
            }
        }
        // CJK chunks: the street is the chunk with a street-type ending, else the chunk right before
        // the house number; earlier chunks are localities (big-endian order)
        let cjk_tok = |k: usize| toks[k].script.is_cjk() || toks[k].script == Script::Hangul;
        if idx.len() > 1 && (idx.iter().all(|&k| cjk_tok(k)) || (ctx.cjk_like(script) && idx.iter().any(|&k| cjk_tok(k)))) {
            idx.retain(|&k| cjk_tok(k));
            let typed = idx.iter().copied().rev().filter(|_| !jp).find(|&k| {
                let s = &toks[k].s;
                s.chars().count() >= 2 && find_suffix(ctx, s, 1, true).map_or(false, |(sfx, _)| sfx.chars().all(|ch| ch.is_alphabetic() && !ch.is_ascii()))
            });
            let pick = typed.or_else(|| match house_idx {
                Some(h) => idx.iter().copied().filter(|&k| k < h).last(),
                None => idx.last().copied(),
            });
            if let Some(p) = pick {
                idx = vec![p];
            }
        }
        if let (Some(&a), Some(&b)) = (idx.first(), idx.last()) {
            street_lo = a;
            street_hi = b;
        }
        // pick the type span at an edge
        let mut type_span: Option<Span> = None;
        if let Some(&last) = idx.last() {
            if let Some(sp) = span_at(last) {
                if sp.has(Class::StreetType) && sp.start + sp.len - 1 == last && idx.len() > sp.len {
                    type_span = Some(sp.clone());
                }
            }
        }
        if type_span.is_none() {
            if let Some(&first) = idx.first() {
                if let Some(sp) = span_at(first) {
                    if sp.has(Class::StreetType) && sp.start == first && idx.len() > sp.len {
                        type_span = Some(sp.clone());
                    }
                }
            }
        }
        if type_span.is_none() {
            // a type in the middle (e.g. "Rua Augusta, loja" or "Main St North")
            for &q in &idx {
                if let Some(sp) = span_at(q) {
                    if sp.has(Class::StreetType) && sp.start == q && idx.len() > sp.len && !sp.has(Class::Title) && !sp.has(Class::Directional) {
                        type_span = Some(sp.clone());
                        break;
                    }
                }
            }
        }
        let mut name_idx: Vec<usize> = idx.clone();
        if type_span.is_none() && idx.len() >= 2 {
            for &edge in [idx[0], *idx.last().unwrap()].iter() {
                if toks[edge].abbr && !toks[edge].num && span_at(edge).map_or(true, |s| !s.has(Class::Title)) {
                    let cs = abbrev_type(ctx, &toks[edge].s);
                    if !cs.is_empty() {
                        c.street_type_word = ascii_key(&toks[edge].s);
                        c.street_type_abbr = true;
                        c.street_type_guess = true;
                        c.street_type_first = edge == idx[0];
                        c.street_type = cs;
                        c.street_type_script = Some(toks[edge].script);
                        name_idx.retain(|&q| q != edge);
                        break;
                    }
                }
            }
        }
        if let Some(sp) = &type_span {
            c.street_type_word = toks[sp.start..sp.start + sp.len].iter().map(|t| ascii_key(&t.s)).collect();
            c.street_type_first = name_idx.iter().any(|&q| q >= sp.start + sp.len);
            c.street_type_abbr = toks[sp.start + sp.len - 1].abbr;
            c.street_type = sp.canons(Class::StreetType);
            c.street_type_script = Some(toks[sp.start].script);
            name_idx.retain(|&q| q < sp.start || q >= sp.start + sp.len);
        }
        // drop stopwords (unless that empties the name) and house-number keywords
        // a compass word that is a stop word in another language is kept ("N" read with no country)
        let content: Vec<usize> = name_idx.iter().copied().filter(|&q| !span_at(q).map_or(false, |s| s.has(Class::Stop) && !s.has(Class::StreetType) && !s.has(Class::Directional))).collect();
        let name_idx = if content.is_empty() { name_idx } else { content };
        let mut toks_out: Vec<(String, Vec<u32>)> = Vec::new();
        let mut abbr_out: Vec<bool> = Vec::new();
        let mut q = 0;
        while q < name_idx.len() {
            let k = name_idx[q];
            let mut canons = Vec::new();
            let mut consumed = 1;
            if let Some(sp) = span_at(k) {
                if sp.start == k && name_idx[q..].len() >= sp.len {
                    for cl in NAME_CLASSES {
                        canons.extend(sp.canons(*cl));
                    }
                    consumed = sp.len;
                }
            }
            let surface: String = name_idx[q..q + consumed].iter().map(|&x| english_ordinal(ctx, &ordinal_digits(&ascii_key(&toks[x].s)))).collect();
            canons.sort();
            canons.dedup();
            toks_out.push((surface, canons));
            abbr_out.push(consumed == 1 && toks[k].abbr);
            q += consumed;
        }
        // concatenated suffix split ("hauptstrasse" -> "haupt" + type) when no separate type
        if type_span.is_none() && !toks_out.is_empty() {
            let lastk = *name_idx.last().unwrap();
            let surf = toks[lastk].s.clone();
            let min_rest = if toks[lastk].script.is_cjk() || toks[lastk].script == Script::Hangul { 1 } else { 3 };
            if let Some((suf, e)) = find_suffix(ctx, &surf, min_rest, false) {
                let tl = surf.chars().count();
                let sl = suf.chars().count();
                let rest: String = surf.chars().take(tl - sl).collect();
                let li = toks_out.len() - 1;
                toks_out[li] = (ascii_key(&rest), Vec::new());
                if let Some(a) = abbr_out.last_mut() {
                    *a = false;
                }
                c.street_type_word = ascii_key(&suf);
                c.street_type_word_chars = sl;
                c.street_type_compound = true;
                c.street_type = vec![e.canon];
                c.street_type_script = Some(toks[lastk].script);
                c.street_type_abbr = toks[lastk].abbr;
            }
        }
        let key: String = toks_out.iter().map(|t| t.0.as_str()).collect();
        let sc = name_idx.first().map(|&k| toks[k].script);
        let mut native: String = name_idx.iter().map(|&k| toks[k].s.as_str()).collect();
        if c.street_type_compound {
            let lastk = *name_idx.last().unwrap();
            let surf = &toks[lastk].s;
            let keep = surf.chars().count().saturating_sub(c.street_type_word_chars);
            let cut: String = surf.chars().skip(keep).collect();
            if native.ends_with(cut.as_str()) {
                native.truncate(native.len() - cut.len());
            }
        }
        c.street = Text { skel: skeleton(&key), key, script: sc, toks: toks_out, abbr: abbr_out, native };
        for &k in &idx {
            used[k] = true;
        }
    }

    // 4a. a short "word number" segment right after the street segment is a unit ("Διαμ. 4", "D:30")
    if let (Some(ss), Some(_)) = (street_seg, &c.house) {
        let s2 = ss + 1;
        let ks: Vec<usize> = (0..n).filter(|&q| toks[q].seg == s2 && !used[q]).collect();
        let nums: Vec<usize> = ks.iter().copied().filter(|&q| toks[q].num).collect();
        let words: Vec<usize> = ks.iter().copied().filter(|&q| !toks[q].num).collect();
        let letters: usize = words.iter().map(|&q| toks[q].s.chars().count()).sum();
        let more_after = (0..n).any(|q| toks[q].seg > s2);
        // a long number is a postcode after a town ("Askar 0000", "Areal 25845-000"), not a unit
        let long_num = nums.first().map_or(false, |&q| toks[q].s.chars().filter(|ch| ch.is_ascii_digit()).count() >= 4);
        // an Irish postal district is a town and its district number, not a unit ("Dublin 2", "Dublin 6W")
        let postal_district = ctx.cc == Some("IE") && nums.len() == 1 && words.len() == 1 && is_dublin_district(&toks[words[0]].s, &toks[nums[0]].s);
        if nums.len() == 1 && words.len() == 1 && letters <= 6 && words[0] < nums[0] && more_after && !long_num && !postal_district {
            c.units.push(format!("u{}", norm_number(&toks[nums[0]].s)));
            c.unit_words.push(ks.iter().map(|&q| toks[q].s.as_str()).collect::<Vec<_>>().join(" "));
            for q in ks {
                used[q] = true;
            }
        }
    }

    // 4b. leftover alphanumeric codes outside the street: postcode-like even when the country
    // regex rejects them (sites and callers write malformed codes)
    if c.postcode.is_none() {
        let cand: Vec<usize> = (0..n)
            .filter(|&q| !used[q] && Some(toks[q].seg) != street_seg && toks[q].s.is_ascii() && toks[q].s.chars().filter(|ch| ch.is_ascii_digit()).count() >= 3)
            .collect();
        if let Some(&k0) = cand.first() {
            let mut ks = vec![k0];
            let mut j = k0 + 1;
            let is_cc = |t: &str| ctx.data.all_country_names.contains_key(t) || ctx.cc.map_or(false, |cc| cc.eq_ignore_ascii_case(t));
            while j < n && toks[j].seg == toks[k0].seg && !used[j] && !is_cc(&toks[j].s) && toks[j].s.is_ascii() && toks[j].s.len() <= 4 && toks[j].s.chars().all(|ch| ch.is_ascii_alphanumeric()) && (toks[j].s.chars().any(|ch| ch.is_ascii_digit()) || toks[j].s.len() <= 2) && ks.len() < 3 {
                ks.push(j);
                j += 1;
            }
            // a letter run before the code ("k 1a" style splits of "K1A")
            if k0 > 0 && !used[k0 - 1] && toks[k0 - 1].seg == toks[k0].seg && toks[k0 - 1].s.len() <= 2 && toks[k0 - 1].s.chars().all(|ch| ch.is_ascii_alphabetic()) {
                let prev_is_region = ctx.cc.and_then(|cc| ctx.data.countries.get(cc)).map_or(false, |co| co.regions.contains_key(&toks[k0 - 1].s));
                if !prev_is_region {
                    ks.insert(0, k0 - 1);
                }
            }
            let mut pc: String = ks.iter().map(|&k| toks[k].s.as_str()).collect::<String>().chars().filter(|ch| ch.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase();
            // a ZIP+4 is its ZIP ("90000-1234")
            if ctx.cc == Some("US") && pc.len() == 9 && pc.chars().all(|ch| ch.is_ascii_digit()) {
                pc.truncate(5);
            }
            c.postcode = Some(pc);
            for k in ks {
                used[k] = true;
            }
        }
    }

    // 5. other segments: country, region, locality, business
    let data = ctx.data;
    let country = ctx.cc.and_then(|cc| data.countries.get(cc));
    let mut seg_texts: Vec<(usize, Text, bool, bool)> = Vec::new(); // (first token, text, is_region, is_country)
    for s in 0..nseg {
        let ks: Vec<usize> = (0..n).filter(|&q| toks[q].seg == s && !used[q]).collect();
        if ks.is_empty() {
            continue;
        }
        // CJK runs: chunk tokens are separate localities
        let groups: Vec<Vec<usize>> = if (ks.iter().all(|&k| toks[k].script.is_cjk() || toks[k].script == Script::Hangul || toks[k].num) || ctx.cjk_like(script)) && ks.len() > 1 {
            ks.iter().map(|&k| vec![k]).collect()
        } else {
            vec![ks]
        };
        for g in groups {
            let words: Vec<usize> = g.iter().copied().filter(|&k| !toks[k].num || (toks[k].s.chars().any(|ch| ch.is_alphabetic()) && ordinal_digits(&toks[k].s) != toks[k].s)).collect();
            if words.iter().all(|&k| toks[k].num) {
                continue;
            }
            let content: Vec<usize> = words.iter().copied().filter(|&k| !span_at(k).map_or(false, |s| s.has(Class::Stop))).collect();
            let words = if content.is_empty() { words } else { content };
            let key: String = words.iter().map(|&k| ascii_key(&toks[k].s)).collect();
            let full_key: String = g.iter().map(|&k| ascii_key(&toks[k].s)).collect();
            let mut is_country = data.all_country_names.contains_key(&key) || country.map_or(false, |c| c.names.contains(&key));
            let mut is_region = country.map_or(false, |c| c.regions.contains_key(&key) || c.regions.contains_key(&full_key));
            // CJK admin suffixes: prefecture/province level are regions
            if words.len() == 1 && toks[words[0]].script.is_cjk() {
                let last = toks[words[0]].s.chars().last().unwrap();
                if matches!(last, '都' | '道' | '府' | '県' | '省') && toks[words[0]].s.chars().count() >= 2 {
                    is_region = true;
                }
            }
            // a region of the hinted country is that region, not a country of the same name ("Georgia"
            // with a US hint)
            if is_country && is_region && country.map_or(false, |c| !c.names.contains(&key)) {
                is_country = false;
            }
            // strip a region's words at the end of a locality ("Springfield IL", "Springfield New
            // York"), keeping it as the town's region
            let mut text_words = words.clone();
            // a state code in capitals as the last word of the town's part, of a country that writes
            // states after the town ("Exampleton IL", "Exampleton ON"), even one that is also a
            // short word ("ON", "IN", "OR"): with no country, or one of those countries given
            let last = *g.last().unwrap();
            let code = ascii_key(&toks[last].s);
            let states_here = |cc: &str| data.countries.get(cc).map_or(false, |co| co.regions.contains_key(&code));
            let code_country = match ctx.cc {
                None => ["US", "CA", "AU"].iter().any(|cc| states_here(cc)),
                Some(cc) => matches!(cc, "US" | "CA" | "AU") && states_here(cc),
            };
            // nothing but numbers (a postcode) after it, and written in capitals
            let at_end = toks[last + 1..].iter().all(|t| t.s.chars().any(|ch| ch.is_ascii_digit()));
            let capital = code.len() == 2 && text.contains(&format!(" {}", code.to_uppercase()));
            if !is_region && !is_country && g.len() > 1 && code_country && at_end && capital && c.town_region.is_none() {
                text_words.retain(|&k| k != last);
                c.town_region = Some(code.clone());
            }
            if !is_region && !is_country && text_words.len() > 1 && c.town_region.is_none() {
                if let Some(co) = country {
                    // where states follow the town (the United States, Canada, Australia), a state's
                    // name of two or three words too, kept as the town's region
                    let states = matches!(ctx.cc, Some("US" | "CA" | "AU"));
                    for l in (1..=(if states { 3 } else { 1 }).min(text_words.len() - 1)).rev() {
                        let tail: String = text_words[text_words.len() - l..].iter().map(|&k| ascii_key(&toks[k].s)).collect();
                        if co.regions.contains_key(&tail) && (l == 1 || tail.len() >= 6) {
                            text_words.truncate(text_words.len() - l);
                            if states {
                                c.town_region = Some(tail);
                            }
                            break;
                        }
                    }
                }
            }
            // strip a trailing country name left in a locality ("trondheim norway")
            if !is_country && text_words.len() > 1 {
                for l in (1..=3.min(text_words.len() - 1)).rev() {
                    let tail: String = text_words[text_words.len() - l..].iter().map(|&k| ascii_key(&toks[k].s)).collect();
                    if data.all_country_names.contains_key(&tail) || country.map_or(false, |c| c.names.contains(&tail)) {
                        text_words.truncate(text_words.len() - l);
                        c.country = true;
                        break;
                    }
                }
            }
            let mut tk: Vec<(String, Vec<u32>)> = text_words.iter().map(|&k| (ordinal_digits(&ascii_key(&toks[k].s)), Vec::new())).collect();
            let mut key: String = tk.iter().map(|t| t.0.as_str()).collect();
            // a country's name stays the country's ("Mexico" is not Mexico City)
            if let (false, Some(canon)) = (is_country, data.exonyms.get(&key)) {
                key = canon.clone();
                tk = vec![(key.clone(), Vec::new())];
            }
            let sc = text_words.first().map(|&k| toks[k].script);
            let ab: Vec<bool> = text_words.iter().map(|&k| toks[k].abbr).collect();
            let native: String = text_words.iter().map(|&k| toks[k].s.as_str()).collect();
            let t = Text { skel: skeleton(&key), key, script: sc, toks: tk, abbr: ab, native };
            seg_texts.push((g[0], t, is_region, is_country));
        }
    }
    // a country's name followed by another place is a town named after it ("Lebanon, PA", "Mexico,
    // MO"); CJK addresses write the country first
    for i in 0..seg_texts.len() {
        let latin = seg_texts[i].1.script.map_or(true, |s| !s.is_cjk() && s != Script::Hangul);
        let at = seg_texts[i].0;
        if seg_texts[i].3 && latin && seg_texts.iter().any(|x| x.0 > at && !x.3) {
            seg_texts[i].3 = false;
        }
    }
    let ss = street_seg.unwrap_or(usize::MAX);
    if street_lo == usize::MAX {
        if let Some(h) = house_idx {
            street_lo = h;
            street_hi = h;
        }
    }
    let before: Vec<&(usize, Text, bool, bool)> = seg_texts.iter().filter(|x| street_lo != usize::MAX && x.0 < street_lo).collect();
    let after: Vec<&(usize, Text, bool, bool)> = seg_texts.iter().filter(|x| street_lo == usize::MAX || x.0 > street_hi).collect();
    let cjk = matches!(script, Some(Script::Han) | Some(Script::Kana) | Some(Script::Hangul));
    // "Плевен, Трифон Кунев 3Б": where towns are often written first, a short segment before the
    // street with nothing after it is the town
    let town_first_cc = matches!(ctx.cc, Some("RU" | "UA" | "BY" | "KZ" | "BG" | "KG" | "UZ" | "AM" | "GE" | "MD"));
    let town_first = town_first_cc && after.is_empty() && before.iter().all(|x| x.1.toks.len() <= 3 && !x.1.key.chars().any(|ch| ch.is_ascii_digit()));
    let big_endian = street_seg.is_some()
        && !before.is_empty()
        && (cjk
            || town_first
            || before.iter().any(|x| x.2 || x.3)
            || (after.iter().all(|x| x.2 || x.3) && postcode_span.map_or(false, |(k, _)| toks[k].seg < ss)));
    let mut region_texts = Vec::new();
    for x in &seg_texts {
        if x.3 {
            c.country = true;
            c.countries.push(x.1.key.clone());
        } else if x.2 {
            c.regions.push(x.1.key.clone());
            region_texts.push(x.1.clone());
        }
    }
    if big_endian {
        for x in before.iter().rev() {
            if !x.2 && !x.3 {
                // Latin words inside a CJK address are a business name ("洪 Bakery")
                if cjk && x.1.script.map_or(false, |s| s == Script::Latin) {
                    c.business.push(x.1.clone());
                } else {
                    c.localities.push(x.1.clone());
                }
            }
        }
        for x in &after {
            if !x.2 && !x.3 {
                c.localities.push(x.1.clone());
            }
        }
    } else {
        for x in &before {
            if !x.2 && !x.3 {
                c.business.push(x.1.clone());
            }
        }
        for x in &after {
            if !x.2 && !x.3 {
                c.localities.push(x.1.clone());
            }
        }
    }
    // regions double as localities (city-states, cantons named like their capital) after real ones
    c.localities.extend(region_texts);
    c.city = c.localities.first().cloned();
    // a ZIP+4 code alone ("60000-1234") is a postcode, not a house number
    if matches!(ctx.cc, None | Some("US")) && c.street.is_empty() && c.localities.is_empty() && c.postcode.is_none() {
        if let Some(h) = c.house.clone() {
            let b = h.as_bytes();
            if b.len() == 10 && b[5] == b'-' && b.iter().enumerate().all(|(i, x)| i == 5 || x.is_ascii_digit()) {
                c.postcode = Some(h[..5].to_string());
                c.house = None;
            }
        }
    }
    // with no country and no commas, a street read to the end ("1 Example St Exampleton IL": "St"
    // may begin a town, "IL" may be a word) still ends in a state code in capitals: keep it as the
    // town's region, so it is compared as "1 Example St, Exampleton IL" would be
    if ctx.cc.is_none() && c.town_region.is_none() && c.localities.is_empty() && !text.contains(',') && c.house.is_some() {
        let words: Vec<&str> = text.split_whitespace().collect();
        let last = words.last().copied().unwrap_or("");
        let code = last.to_ascii_lowercase();
        let state = ["US", "CA", "AU"].iter().any(|cc| ctx.data.countries.get(*cc).map_or(false, |co| co.regions.contains_key(&code)));
        if words.len() >= 4 && last.len() == 2 && last.chars().all(|ch| ch.is_ascii_uppercase()) && state {
            c.town_region = Some(code);
        }
    }
    c
}

impl<'a> Ctx<'a> {
    fn cjk_like(&self, s: Option<Script>) -> bool {
        matches!(s, Some(Script::Han) | Some(Script::Kana) | Some(Script::Hangul))
    }
}

/// Dublin's postal districts, written as the town and the district's number: 1 to 24, and 6W.
fn is_dublin_district(word: &str, number: &str) -> bool {
    if !word.eq_ignore_ascii_case("dublin") {
        return false;
    }
    let n = number.trim().to_ascii_lowercase();
    n == "6w" || n.parse::<u32>().map_or(false, |v| (1..=24).contains(&v))
}

