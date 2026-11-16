//! Embedded dictionaries (libpostal) and country data (libaddressinput, OpenCage), parsed once
//! into immutable lookup tables owned by a `Matcher`.
use crate::fold::{all_han, ascii_key, fold, han_canon};
use crate::token::tokenize_plain;
use regex_lite::Regex;
use std::collections::HashMap;
use std::borrow::Cow;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::OnceLock;

/// Small fast hasher for short string keys (FxHash scheme). Not DoS-resistant; keys are ours.
#[derive(Default, Clone, Copy)]
pub struct Fx(u64);
impl Hasher for Fx {
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut v = 0u64;
            for (i, b) in chunk.iter().enumerate() {
                v |= (*b as u64) << (8 * i);
            }
            self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        }
    }
    fn write_u8(&mut self, i: u8) {
        self.0 = (self.0.rotate_left(5) ^ i as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}
pub type FxBuild = BuildHasherDefault<Fx>;


#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Class {
    StreetType,
    Directional,
    UnitNum,
    UnitAlone,
    LevelNum,
    LevelAlone,
    PoBox,
    Stop,
    HouseKw,
    Synonym,
    SuffixSep,
    SuffixInsep,
    Title,
    Qualifier,
    Building,
    Company,
    Place,
    Entrance,
    Staircase,
    NoNumber,
}

impl Class {
    fn from_file(s: &str) -> Option<Class> {
        Some(match s {
            "street_types" => Class::StreetType,
            "directionals" => Class::Directional,
            "unit_types_numbered" => Class::UnitNum,
            "unit_types_standalone" => Class::UnitAlone,
            "level_types_numbered" => Class::LevelNum,
            "level_types_standalone" | "level_types_basement" | "level_types_mezzanine" => Class::LevelAlone,
            "post_office" => Class::PoBox,
            "stopwords" => Class::Stop,
            "house_numbers" | "number" => Class::HouseKw,
            "synonyms" => Class::Synonym,
            "concatenated_suffixes_separable" => Class::SuffixSep,
            "concatenated_suffixes_inseparable" => Class::SuffixInsep,
            "personal_titles" => Class::Title,
            "qualifiers" => Class::Qualifier,
            "building_types" => Class::Building,
            "company_types" => Class::Company,
            "place_names" => Class::Place,
            "entrances" => Class::Entrance,
            "staircases" => Class::Staircase,
            "no_number" => Class::NoNumber,
            _ => return None,
        })
    }
    pub fn from_u8(v: u8) -> Class {
        const ALL: [Class; 20] = [
            Class::StreetType, Class::Directional, Class::UnitNum, Class::UnitAlone, Class::LevelNum, Class::LevelAlone, Class::PoBox, Class::Stop, Class::HouseKw, Class::Synonym,
            Class::SuffixSep, Class::SuffixInsep, Class::Title, Class::Qualifier, Class::Building, Class::Company, Class::Place, Class::Entrance, Class::Staircase, Class::NoNumber,
        ];
        ALL[v as usize]
    }
    pub fn bit(self) -> u32 {
        1 << (self as u8)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub class: Class,
    pub lang: u8,
    pub canon: u32,
}

pub struct Country {
    pub langs: Vec<u8>,
    pub zip_src: String,
    zip_re: OnceLock<Option<Regex>>,
    /// folded ASCII keys of region names and codes, each to its region (an index in `region_info`):
    /// a region's names, codes and names in other scripts share one index
    pub regions: HashMap<String, u16>,
    /// short names of regions, without a kind word ("taichung" for "Taichung City", "jiangsu" for
    /// "Jiangsu Sheng"), used only to compare a place name with a region, never in parsing
    pub region_short: HashMap<String, u16>,
    /// keys that name more than one region (two provinces with one toneless reading, "shanxi" for
    /// 山西省 and 陕西省): every region they may name
    pub region_shared: HashMap<String, Vec<u16>>,
    pub region_info: Vec<RegionInfo>,
    pub names: Vec<String>,
}

impl Country {
    /// The regions a key may name: one, several when the key is shared, or none.
    pub fn region_ids(&self, key: &str) -> Vec<u16> {
        if let Some(ids) = self.region_shared.get(key) {
            return ids.clone();
        }
        self.regions.get(key).or_else(|| self.region_short.get(key)).map_or(Vec::new(), |&id| vec![id])
    }
}

/// One region of a country.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegionInfo {
    /// The lists that name it: 1 libaddressinput, 2 OpenCage. Two lists may name regions of
    /// different levels (provinces in one, regions in the other), so only regions named by one list
    /// are compared with each other.
    pub sources: u8,
    /// The region is a city ("台南市", "Taichung City", a federal city).
    pub city: bool,
}

/// Words that name a region's kind in its romanized or English name.
const ADMIN_WORDS: &[&str] = &[
    "sheng", "shi", "zizhiqu", "xian", "oblast", "kray", "krai", "respublika", "okrug", "avtonomnyy", "avtonomnaya", "county", "province", "prefecture",
    "region", "state", "district", "city", "do", "si", "gun", "ken", "fu", "to", "teukbyeolsi", "gwangyeoksi", "teukbyeoljachisi", "teukbyeoljachido",
];
/// Kind words that make the region a city.
const CITY_WORDS: &[&str] = &["city", "shi", "si", "teukbyeolsi", "gwangyeoksi", "teukbyeoljachisi"];
/// Han and Hangul suffixes of a region's kind.
const ADMIN_SUFFIXES: &[char] = &['省', '市', '縣', '县', '府', '都', '道', '区', '州', '시', '도'];

/// Regions from groups of names ("a:" libaddressinput, "y:" OpenCage), joined where they share a
/// name or code, each with its short names (without a kind word: "Taichung" for "Taichung City",
/// "江苏" for "江苏省") and whether it is a city.
fn build_regions(field: &str) -> (HashMap<String, u16>, HashMap<String, u16>, HashMap<String, Vec<u16>>, Vec<RegionInfo>) {
    let groups: Vec<(u8, Vec<String>)> = field
        .split('~')
        .filter_map(|g| g.split_once(':'))
        .map(|(fam, names)| (if fam == "a" { 1 } else { 2 }, names.split('|').filter(|n| !n.is_empty()).map(|n| n.to_string()).collect()))
        .collect();
    let mut parent: Vec<usize> = (0..groups.len()).collect();
    fn root(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    // two groups of one list are two regions, even when their names read alike (山西省 and 陕西省
    // are both "shanxisheng"); groups of the two lists are joined on a name or code that names one
    // group in each list
    let mut by_key: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, (_, names)) in groups.iter().enumerate() {
        for n in names {
            let k = concat_key(n);
            if !k.is_empty() {
                let v = by_key.entry(k).or_default();
                if !v.contains(&i) {
                    v.push(i);
                }
            }
        }
    }
    let mut keys: Vec<&String> = by_key.keys().collect();
    keys.sort();
    for k in keys {
        let gs = &by_key[k];
        let (a, y): (Vec<usize>, Vec<usize>) = gs.iter().partition(|&&g| groups[g].0 == 1);
        if a.len() == 1 && y.len() == 1 {
            let (ra, ry) = (root(&mut parent, a[0]), root(&mut parent, y[0]));
            parent[ra] = ry;
        }
    }
    let words = |n: &str| -> Vec<String> { tokenize_plain(&fold(n)).iter().map(|t| ascii_key(t)).filter(|t| !t.is_empty()).collect() };
    let mut ids: HashMap<usize, u16> = HashMap::new();
    let mut info: Vec<RegionInfo> = Vec::new();
    let mut members: Vec<Vec<String>> = Vec::new();
    for i in 0..groups.len() {
        let r = root(&mut parent, i);
        let id = *ids.entry(r).or_insert_with(|| {
            info.push(RegionInfo::default());
            members.push(Vec::new());
            (info.len() - 1) as u16
        });
        info[id as usize].sources |= groups[i].0;
        members[id as usize].extend(groups[i].1.iter().cloned());
    }
    // a region with no kind word, in a country whose regions mostly have one, is a city (a federal city)
    let has_kind = |names: &[String]| names.iter().any(|n| n.ends_with(ADMIN_SUFFIXES) || words(n).last().map_or(false, |w| ADMIN_WORDS.contains(&w.as_str())));
    let with_kind = members.iter().filter(|m| has_kind(m)).count();
    let mostly = !members.is_empty() && with_kind * 5 >= members.len() * 4;
    let mut named: HashMap<String, Vec<u16>> = HashMap::new();
    for (id, names) in members.iter().enumerate() {
        let city = names.iter().any(|n| n.ends_with('市') || n.ends_with("광역시") || n.ends_with("특별시") || words(n).last().map_or(false, |w| CITY_WORDS.contains(&w.as_str())));
        info[id].city = city || (mostly && !has_kind(names));
        for n in names {
            let k = concat_key(n);
            if !k.is_empty() {
                let v = named.entry(k).or_default();
                if !v.contains(&(id as u16)) {
                    v.push(id as u16);
                }
            }
        }
    }
    let mut regions: HashMap<String, u16> = HashMap::new();
    let mut shared: HashMap<String, Vec<u16>> = HashMap::new();
    for (k, ids) in named {
        regions.insert(k.clone(), ids[0]);
        if ids.len() > 1 {
            shared.insert(k, ids);
        }
    }
    // short names, where no region already has them as a name
    let mut short_all: HashMap<String, Vec<u16>> = HashMap::new();
    for (id, names) in members.iter().enumerate() {
        for n in names {
            let w = words(n);
            let mut short: Vec<String> = Vec::new();
            if w.len() >= 2 && ADMIN_WORDS.contains(&w[w.len() - 1].as_str()) {
                short.push(w[..w.len() - 1].concat());
            }
            let chars: Vec<char> = n.chars().collect();
            if chars.len() >= 3 && n.ends_with(ADMIN_SUFFIXES) {
                short.push(concat_key(&chars[..chars.len() - 1].iter().collect::<String>()));
            }
            // a Han name also by its characters, which tell apart regions that read alike
            // ("山西省" and "陕西省"), with and without its kind suffix
            if all_han(n) {
                let h: Vec<char> = han_canon(n).chars().collect();
                short.push(h.iter().collect());
                if h.len() >= 2 && n.ends_with(ADMIN_SUFFIXES) {
                    short.push(h[..h.len() - 1].iter().collect());
                }
            }
            for k in short {
                if k.len() >= 2 && !regions.contains_key(&k) {
                    let v = short_all.entry(k).or_default();
                    if !v.contains(&(id as u16)) {
                        v.push(id as u16);
                    }
                }
            }
        }
    }
    let mut short_names: HashMap<String, u16> = HashMap::new();
    for (k, ids) in short_all {
        short_names.insert(k.clone(), ids[0]);
        if ids.len() > 1 {
            shared.insert(k, ids);
        }
    }
    (regions, short_names, shared, info)
}

pub struct Data {
    pub langs: Vec<String>,
    /// phrase -> range in `entries`
    pub phrase_index: HashMap<Cow<'static, str>, (u32, u32), FxBuild>,
    pub entries: Vec<Entry>,
    pub max_phrase: usize,
    pub canon_keys: Vec<String>,
    /// (suffix surface, entry) for concatenated suffixes and CJK/Hangul street types
    pub suffixes: Vec<(String, Entry)>,
    pub countries: HashMap<String, Country>,
    pub all_country_names: HashMap<String, ()>,
    /// single-word street types (folded surface) for abbreviation matching ("gt." -> "gata")
    pub type_words: Vec<(String, Entry)>,
    /// suffix surface -> entries (longest-first lookup by char length)
    pub suffix_map: HashMap<String, Vec<Entry>>,
    pub max_suffix: usize,
    /// type words grouped by first char, for abbreviation lookup
    pub type_by_first: HashMap<char, Vec<(String, Entry)>>,
    /// canonical id -> ASCII keys of its single-word type spellings
    pub type_keys_by_canon: HashMap<u32, Vec<String>>,
    /// city name key -> canonical key (endonyms and exonyms)
    pub exonyms: HashMap<String, String>,
}

/// Country names per ISO code: English, native and common variants (hand list, synthetic-safe).
const COUNTRY_NAMES: &[(&str, &str)] = &[
    ("AE", "united arab emirates|uae|u.a.e.|emirates|الإمارات العربية المتحدة|الإمارات"),
    ("AR", "argentina"),
    ("AT", "austria|österreich|oesterreich"),
    ("AU", "australia"),
    ("BE", "belgium|belgique|belgië|belgie|belgien"),
    ("BR", "brazil|brasil"),
    ("CA", "canada"),
    ("CH", "switzerland|schweiz|suisse|svizzera|suisse"),
    ("CN", "china|中国|中华人民共和国|people's republic of china|prc"),
    ("CZ", "czech republic|czechia|česko|česká republika"),
    ("DE", "germany|deutschland|bundesrepublik deutschland|brd"),
    ("DK", "denmark|danmark"),
    ("EG", "egypt|مصر|misr"),
    ("ES", "spain|españa|espana|espanya"),
    ("FI", "finland|suomi"),
    ("FR", "france|république française"),
    ("GB", "united kingdom|uk|u.k.|great britain|england|scotland|wales|northern ireland|gb"),
    ("GR", "greece|ελλάδα|ελλαδα|ellada|hellas"),
    ("IE", "ireland|éire|eire"),
    ("IL", "israel|ישראל|yisrael"),
    ("IN", "india|भारत|bharat"),
    ("IT", "italy|italia"),
    ("JP", "japan|日本|nihon|nippon|日本国"),
    ("KR", "south korea|korea|republic of korea|대한민국|한국|hanguk|daehan minguk"),
    ("MX", "mexico|méxico|estados unidos mexicanos"),
    ("NL", "netherlands|nederland|the netherlands|holland"),
    ("NO", "norway|norge|noreg"),
    ("NZ", "new zealand|aotearoa"),
    ("PL", "poland|polska"),
    ("PT", "portugal"),
    ("RU", "russia|россия|российская федерация|rossiya|russian federation|rossiia"),
    ("SA", "saudi arabia|ksa|المملكة العربية السعودية|السعودية"),
    ("SE", "sweden|sverige"),
    ("SG", "singapore|新加坡|singapura"),
    ("TH", "thailand|ประเทศไทย|ไทย|prathet thai"),
    ("TR", "turkey|türkiye|turkiye"),
    ("TW", "taiwan|臺灣|台灣|台湾|中華民國"),
    ("UA", "ukraine|україна|ukraina"),
    ("US", "united states|usa|u.s.a.|us|u.s.|united states of america|america"),
    ("VN", "vietnam|viet nam|việt nam"),
    ("ZA", "south africa|suid-afrika"),
];

pub fn phrase_key(s: &str) -> String {
    let toks = tokenize_plain(&fold(s));
    toks.join(" ")
}

pub fn concat_key(s: &str) -> String {
    tokenize_plain(&fold(s)).iter().map(|t| ascii_key(t)).collect::<Vec<_>>().join("")
}

impl Country {
    /// The libaddressinput postcode regex, compiled on first use.
    pub fn zip(&self) -> Option<&Regex> {
        self.zip_re
            .get_or_init(|| if self.zip_src.is_empty() { None } else { Regex::new(&format!("^(?i:{})$", self.zip_src)).ok() })
            .as_ref()
    }
}

impl Data {
    /// Build from the raw dictionary dump (run at build time by build.rs).
    pub fn build_from_raw(raw: &str) -> Data {
        let mut langs: Vec<String> = Vec::new();
        let mut phrases: HashMap<String, Vec<Entry>> = HashMap::new();
        let mut canon_ids: HashMap<String, u32> = HashMap::new();
        let mut canon_keys: Vec<String> = Vec::new();
        let mut suffixes = Vec::new();
        let mut countries = HashMap::new();
        let mut max_phrase = 1;
        let mut all_country_names = HashMap::new();
        let mut country_rows = Vec::new();
        let mut type_words = Vec::new();
        let mut exonyms = HashMap::new();
        let mut prev_native: Option<(u8, Class, u32)> = None;
        for line in raw.lines() {
            let mut it = line.split('\t');
            match it.next() {
                Some("D") => {
                    let lang = it.next().unwrap();
                    let class = match Class::from_file(it.next().unwrap()) {
                        Some(c) => c,
                        None => continue,
                    };
                    let body = it.next().unwrap_or("");
                    let li = match langs.iter().position(|l| l == lang) {
                        Some(i) => i,
                        None => {
                            langs.push(lang.to_string());
                            langs.len() - 1
                        }
                    } as u8;
                    let mut alts = body.split('|');
                    let canon_s = alts.next().unwrap_or("");
                    let ck = concat_key(canon_s);
                    if ck.is_empty() {
                        continue;
                    }
                    let cid = *canon_ids.entry(ck.clone()).or_insert_with(|| {
                        canon_keys.push(ck.clone());
                        (canon_keys.len() - 1) as u32
                    });
                    let e = Entry { class, lang: li, canon: cid };
                    // libpostal lists a native-script line followed by its romanization
                    // ("улица|ул" then "ulitsa|ulica|ul"): the romanized line is the same word
                    let native = !canon_s.is_ascii() && canon_s.chars().any(|ch| crate::fold::script_of(ch) != crate::fold::Script::Latin && ch.is_alphabetic());
                    let alias = match prev_native {
                        Some((pl, pc, pid)) if pl == li && pc == class && !native => Some(pid),
                        _ => None,
                    };
                    prev_native = if native { Some((li, class, cid)) } else { None };
                    let all_alts: Vec<&str> = std::iter::once(canon_s).chain(alts).collect();
                    if let Some(aid) = alias {
                        let e2 = Entry { class, lang: li, canon: aid };
                        for alt in &all_alts {
                            let pk = phrase_key(alt);
                            if !pk.is_empty() {
                                phrases.entry(pk).or_default().push(e2);
                            }
                        }
                    }
                    for alt in all_alts {
                        let pk = phrase_key(alt);
                        if pk.is_empty() {
                            continue;
                        }
                        let n = pk.split(' ').count();
                        max_phrase = max_phrase.max(n);
                        if matches!(class, Class::SuffixSep | Class::SuffixInsep) || (class == Class::StreetType && !alt.is_ascii() && n == 1) {
                            let s = pk.replace(' ', "");
                            if !s.is_empty() {
                                suffixes.push((s, e));
                            }
                        }
                        if class == Class::StreetType && n == 1 && pk.chars().count() >= 3 {
                            type_words.push((pk.clone(), e));
                        }
                        let v = phrases.entry(pk).or_default();
                        if !v.iter().any(|x| x.class == e.class && x.lang == e.lang && x.canon == e.canon) {
                            v.push(e);
                        }
                    }
                }
                Some("C") => country_rows.push(line.to_string()),
                Some("E") => {
                    let body = it.next().unwrap_or("");
                    let mut alts = body.split('|');
                    let canon = concat_key(alts.next().unwrap_or(""));
                    for a in body.split('|') {
                        exonyms.insert(concat_key(a), canon.clone());
                    }
                }
                _ => {}
            }
        }
        for row in country_rows {
            let f: Vec<&str> = row.split('\t').collect();
            let cc = f[1].to_string();
            let mut lv: Vec<u8> = f[2]
                .split(',')
                .map(|l| match l.trim() {
                    "jp" => "ja",
                    "no" | "nn" => "nb",
                    "tl" => "fil",
                    x => x,
                })
                .filter_map(|l| langs.iter().position(|x| x == l).map(|i| i as u8))
                .collect();

            // an empty list means every dictionary is searched
            if let Some(en) = langs.iter().position(|x| x == "en").filter(|_| !lv.is_empty()) {
                if !lv.contains(&(en as u8)) {
                    lv.push(en as u8);
                }
            }
            let zip_src = f[3].to_string();
            let (regions, mut region_short, region_shared, region_info) = build_regions(f.get(4).unwrap_or(&""));
            // a region that is a city is also compared under the city's other names ("moskva" is
            // also "moscow")
            for (k, &id) in &regions {
                if let Some(canon) = exonyms.get(k).filter(|c| !regions.contains_key(*c) && !region_shared.contains_key(*c)) {
                    region_short.entry(canon.clone()).or_insert(id);
                }
            }
            let mut names = Vec::new();
            // the country's own English name counts only for that country ("Georgia" is also a state)
            if let Some(n) = f.get(5).filter(|n| !n.is_empty()) {
                names.push(concat_key(n));
            }
            for (c, ns) in COUNTRY_NAMES {
                if *c == cc {
                    for n in ns.split('|') {
                        let k = concat_key(n);
                        all_country_names.insert(k.clone(), ());
                        names.push(k);
                    }
                }
            }
            names.push(cc.to_ascii_lowercase());
            countries.insert(cc, Country { langs: lv, zip_src, zip_re: OnceLock::new(), regions, region_short, region_shared, region_info, names });
        }
        suffixes.sort_by(|a, b| b.0.chars().count().cmp(&a.0.chars().count()));
        let mut suffix_map: HashMap<String, Vec<Entry>> = HashMap::new();
        let mut max_suffix = 0;
        for (sfx, e) in &suffixes {
            max_suffix = max_suffix.max(sfx.chars().count());
            suffix_map.entry(sfx.clone()).or_default().push(*e);
        }
        let mut type_by_first: HashMap<char, Vec<(String, Entry)>> = HashMap::new();
        let mut type_keys_by_canon: HashMap<u32, Vec<String>> = HashMap::new();
        for (w, e) in &type_words {
            if let Some(f) = w.chars().next() {
                type_by_first.entry(f).or_default().push((w.clone(), *e));
            }
            let k = ascii_key(w);
            let v = type_keys_by_canon.entry(e.canon).or_default();
            if !v.contains(&k) {
                v.push(k);
            }
        }
        let mut keys: Vec<String> = phrases.keys().cloned().collect();
        keys.sort();
        let mut phrase_index = HashMap::with_hasher(FxBuild::default());
        let mut entries = Vec::new();
        for k in keys {
            let v = &phrases[&k];
            phrase_index.insert(Cow::Owned(k.clone()), (entries.len() as u32, v.len() as u32));
            entries.extend(v.iter().copied());
        }
        Data { langs, phrase_index, entries, max_phrase: max_phrase.min(5), canon_keys, suffixes, countries, all_country_names, type_words, suffix_map, max_suffix, type_by_first, type_keys_by_canon, exonyms }
    }
}

impl Data {
    /// Compact text form written by build.rs and embedded in the library.
    pub fn serialize(&self) -> String {
        use std::fmt::Write;
        let mut o = String::new();
        for l in &self.langs {
            writeln!(o, "L\t{}", l).unwrap();
        }
        for k in &self.canon_keys {
            writeln!(o, "K\t{}", k).unwrap();
        }
        writeln!(o, "X\t{}", self.max_phrase).unwrap();
        let mut keys: Vec<&Cow<'static, str>> = self.phrase_index.keys().collect();
        keys.sort();
        for k in keys {
            for e in self.lookup(k) {
                writeln!(o, "P\t{}\t{}\t{}\t{}", k, e.class as u8, e.lang, e.canon).unwrap();
            }
        }
        for (sfx, e) in &self.suffixes {
            writeln!(o, "S\t{}\t{}\t{}\t{}", sfx, e.class as u8, e.lang, e.canon).unwrap();
        }
        for (w, e) in &self.type_words {
            writeln!(o, "W\t{}\t{}\t{}\t{}\t{}", w, ascii_key(w), e.class as u8, e.lang, e.canon).unwrap();
        }
        let mut ex: Vec<(&String, &String)> = self.exonyms.iter().collect();
        ex.sort();
        for (k, v) in ex {
            writeln!(o, "E\t{}\t{}", k, v).unwrap();
        }
        let mut names: Vec<&String> = self.all_country_names.keys().collect();
        names.sort();
        for n in names {
            writeln!(o, "N\t{}", n).unwrap();
        }
        let mut ccs: Vec<&String> = self.countries.keys().collect();
        ccs.sort();
        for cc in ccs {
            let c = &self.countries[cc];
            let mut regions: Vec<(&String, &u16)> = c.regions.iter().collect();
            regions.sort();
            let langs: Vec<String> = c.langs.iter().map(|l| l.to_string()).collect();
            let regions: Vec<String> = regions.iter().map(|(k, id)| format!("{}={}", k, id)).collect();
            let info: Vec<String> = c.region_info.iter().map(|r| format!("{}.{}", r.sources, r.city as u8)).collect();
            let mut short: Vec<(&String, &u16)> = c.region_short.iter().collect();
            short.sort();
            let short: Vec<String> = short.iter().map(|(k, id)| format!("{}={}", k, id)).collect();
            let mut shared: Vec<(&String, &Vec<u16>)> = c.region_shared.iter().collect();
            shared.sort();
            let shared: Vec<String> = shared.iter().map(|(k, ids)| format!("{}={}", k, ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","))).collect();
            writeln!(o, "C\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}", cc, langs.join(","), c.zip_src, regions.join("~"), c.names.join("~"), info.join("~"), short.join("~"), shared.join("~")).unwrap();
        }
        o
    }

    /// Parse the compact form: no folding or transliteration at load time.
    pub fn deserialize(src: &'static str) -> Data {
        let mut d = Data {
            langs: Vec::new(),
            phrase_index: HashMap::with_hasher(FxBuild::default()),
            entries: Vec::with_capacity(60000),
            max_phrase: 1,
            canon_keys: Vec::new(),
            suffixes: Vec::new(),
            countries: HashMap::new(),
            all_country_names: HashMap::new(),
            type_words: Vec::new(),
            suffix_map: HashMap::new(),
            max_suffix: 0,
            type_by_first: HashMap::new(),
            type_keys_by_canon: HashMap::new(),
            exonyms: HashMap::new(),
        };
        let ent = |c: &str, l: &str, k: &str| Entry { class: Class::from_u8(c.parse().unwrap()), lang: l.parse().unwrap(), canon: k.parse().unwrap() };
        d.phrase_index.reserve(50000);
        for line in src.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f[0] {
                "L" => d.langs.push(f[1].to_string()),
                "K" => d.canon_keys.push(f[1].to_string()),
                "X" => d.max_phrase = f[1].parse().unwrap(),
                "P" => {
                    // lines are sorted by phrase: extend the current range or start a new one
                    let e = ent(f[2], f[3], f[4]);
                    let i = d.entries.len() as u32;
                    d.entries.push(e);
                    match d.phrase_index.get_mut(f[1]) {
                        Some(r) => r.1 += 1,
                        None => {
                            d.phrase_index.insert(Cow::Borrowed(f[1]), (i, 1));
                        }
                    }
                }
                "S" => {
                    let e = ent(f[2], f[3], f[4]);
                    d.max_suffix = d.max_suffix.max(f[1].chars().count());
                    d.suffix_map.entry(f[1].to_string()).or_default().push(e);
                    d.suffixes.push((f[1].to_string(), e));
                }
                "W" => {
                    let e = ent(f[3], f[4], f[5]);
                    if let Some(c) = f[1].chars().next() {
                        d.type_by_first.entry(c).or_default().push((f[1].to_string(), e));
                    }
                    let v = d.type_keys_by_canon.entry(e.canon).or_default();
                    if !v.iter().any(|x| x == f[2]) {
                        v.push(f[2].to_string());
                    }
                    d.type_words.push((f[1].to_string(), e));
                }
                "E" => {
                    d.exonyms.insert(f[1].to_string(), f[2].to_string());
                }
                "N" => {
                    d.all_country_names.insert(f[1].to_string(), ());
                }
                "C" => {
                    let langs = f[2].split(',').filter(|x| !x.is_empty()).map(|x| x.parse().unwrap()).collect();
                    let regions = f[4].split('~').filter_map(|x| x.rsplit_once('=')).map(|(k, id)| (k.to_string(), id.parse().unwrap())).collect();
                    let names = f[5].split('~').filter(|x| !x.is_empty()).map(|x| x.to_string()).collect();
                    let region_info = f.get(6).unwrap_or(&"").split('~').filter_map(|x| x.split_once('.')).map(|(s, c)| RegionInfo { sources: s.parse().unwrap(), city: c == "1" }).collect();
                    let region_short = f.get(7).unwrap_or(&"").split('~').filter_map(|x| x.rsplit_once('=')).map(|(k, id)| (k.to_string(), id.parse().unwrap())).collect();
                    let region_shared = f.get(8).unwrap_or(&"").split('~').filter_map(|x| x.rsplit_once('=')).map(|(k, ids)| (k.to_string(), ids.split(',').filter_map(|i| i.parse().ok()).collect())).collect();
                    d.countries.insert(f[1].to_string(), Country { langs, zip_src: f[3].to_string(), zip_re: OnceLock::new(), regions, region_short, region_shared, region_info, names });
                }
                _ => {}
            }
        }
        d
    }
}

impl Data {
    /// Dictionary entries for a folded phrase (tokens joined by single spaces).
    /// A country's name in any country's list ("chile", "deutschland", "uae").
    pub fn is_country_name(&self, key: &str) -> bool {
        self.all_country_names.contains_key(key) || (key.len() > 2 && self.countries.values().any(|c| c.names.iter().any(|n| n == key)))
    }

    pub fn lookup(&self, key: &str) -> &[Entry] {
        match self.phrase_index.get(key) {
            Some(&(a, n)) => &self.entries[a as usize..(a + n) as usize],
            None => &[],
        }
    }
}
