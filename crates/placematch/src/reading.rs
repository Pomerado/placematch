//! Readings of Han and Kana names, to compare a native-script place name with its romanization
//! ("台中" / "Taichung", "久留米" / "Kurume", "旭町" / "Asahicho").
//!
//! Per-character readings come from the Unihan database (Unicode License v3, see
//! LICENSE-UNICODE): Mandarin pinyin, and Japanese on and kun readings. A romanization matches
//! when it can be split, in order, into one reading for every character. Chinese readings are
//! also accepted in Wade-Giles and Tongyong spellings; Japanese readings in Hepburn or Kunrei
//! spelling, with long vowels written or not, and with the sound changes compound names take
//! (voicing of a later character, a doubled consonant).
use std::collections::HashMap;
use std::sync::OnceLock;

const HAN: &str = include_str!("../data/readings/han.txt");

struct Readings {
    zh: HashMap<char, Vec<String>>,
    ja: HashMap<char, Vec<String>>,
}

fn readings() -> &'static Readings {
    static R: OnceLock<Readings> = OnceLock::new();
    R.get_or_init(|| {
        let mut zh = HashMap::with_capacity(27000);
        let mut ja = HashMap::with_capacity(12000);
        for line in HAN.lines() {
            let mut f = line.split('\t');
            let (Some(c), Some(m), Some(j)) = (f.next(), f.next(), f.next()) else { continue };
            let Some(ch) = c.chars().next() else { continue };
            if !m.is_empty() {
                let mut v: Vec<String> = Vec::new();
                for p in m.split(' ') {
                    for x in zh_variants(p) {
                        if !v.contains(&x) {
                            v.push(x);
                        }
                    }
                }
                zh.insert(ch, v);
            }
            if !j.is_empty() {
                let mut v: Vec<String> = Vec::new();
                for r in j.split(' ') {
                    for x in ja_variants(r) {
                        if !v.contains(&x) {
                            v.push(x);
                        }
                    }
                }
                ja.insert(ch, v);
            }
        }
        Readings { zh, ja }
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Chinese,
    Japanese,
}

/// Languages to try for a country hint: Japanese for JP, Chinese for Chinese-speaking places,
/// both when the country is unknown.
pub fn langs_for(cc: Option<&str>) -> &'static [Lang] {
    match cc {
        Some("JP") => &[Lang::Japanese],
        Some("CN" | "TW" | "HK" | "MO" | "SG") => &[Lang::Chinese],
        None => &[Lang::Chinese, Lang::Japanese],
        _ => &[],
    }
}

const ZH_INITIALS: &[&str] = &["zh", "ch", "sh", "b", "p", "m", "f", "d", "t", "n", "l", "g", "k", "h", "j", "q", "x", "r", "z", "c", "s", "y", "w"];

/// Spellings of a toneless pinyin syllable in Hanyu Pinyin, Wade-Giles (apostrophes dropped) and
/// Tongyong Pinyin, mixed per initial and final as Taiwanese signs often are.
fn zh_variants(p: &str) -> Vec<String> {
    let p = p.trim();
    let ini = ZH_INITIALS.iter().find(|i| p.starts_with(**i) && p.len() > i.len()).copied().unwrap_or("");
    let fin = &p[ini.len()..];
    let inis: Vec<&str> = match ini {
        "b" => vec!["b", "p"],
        "p" => vec!["p"],
        "d" => vec!["d", "t"],
        "t" => vec!["t"],
        "g" => vec!["g", "k"],
        "k" => vec!["k"],
        "zh" => vec!["zh", "ch", "jh", "j"],
        "ch" => vec!["ch"],
        "j" => vec!["j", "ch"],
        "q" => vec!["q", "ch", "c"],
        "x" => vec!["x", "hs", "s", "sh"],
        "r" => vec!["r", "j"],
        "z" => vec!["z", "ts", "tz"],
        "c" => vec!["c", "ts", "tz"],
        "s" => vec!["s", "ss", "sz"],
        other => vec![other],
    };
    let apical = matches!(ini, "zh" | "ch" | "sh" | "r" | "z" | "c" | "s");
    let mut fins: Vec<String> = vec![fin.to_string()];
    let mut add = |f: &str| {
        if !fins.iter().any(|x| x == f) {
            fins.push(f.to_string());
        }
    };
    match fin {
        "i" if apical => {
            add("ih");
            add("u");
            add("y");
            add("");
        }
        "ong" => add("ung"),
        "iong" => {
            add("iung");
            add("yung");
        }
        "ian" => add("ien"),
        "ie" => add("ieh"),
        "e" => {
            add("o");
            add("eh");
        }
        "uo" => add("o"),
        "ui" => add("uei"),
        "iu" => add("iou"),
        "un" => add("uen"),
        "eng" => add("ong"),
        "en" => add("un"),
        "ue" => {
            add("ueh");
            add("ve");
        }
        "u" if matches!(ini, "j" | "q" | "x" | "y") => add("yu"),
        "ou" if ini == "y" => add("u"),
        "an" if ini == "y" => add("en"),
        _ => {}
    }
    let mut out = Vec::new();
    for i in &inis {
        for f in &fins {
            let s = format!("{}{}", i, f);
            if !s.is_empty() && !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

/// Split romaji into morae: consonants and a vowel, or a syllabic n.
fn morae(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let (mut st, mut i) = (0, 0);
    while i < b.len() {
        let c = b[i];
        let syllabic_n = c == b'n' && i == st && (i + 1 >= b.len() || !b"aeiouy".contains(&b[i + 1]));
        i += 1;
        if b"aeiou".contains(&c) || syllabic_n {
            out.push(&s[st..i]);
            st = i;
        }
    }
    if st < b.len() {
        out.push(&s[st..]);
    }
    out
}

/// A Japanese reading as names use it: the reading itself, the stem of a kun reading without its
/// inflected ending ("tomeru" -> "tome", "nagai" -> "naga"), each normalized.
fn ja_variants(r: &str) -> Vec<String> {
    let mut v = vec![ja_norm(r)];
    let m = morae(r);
    if m.len() >= 3 {
        let stem: String = m[..m.len() - 1].concat();
        v.push(ja_norm(&stem));
    }
    if m.len() >= 4 {
        let stem: String = m[..m.len() - 2].concat();
        v.push(ja_norm(&stem));
    }
    v.retain(|x| !x.is_empty());
    v.dedup();
    v
}

/// Romaji to one spelling: Kunrei forms for Hepburn ones, long vowels single, "m" before a labial
/// as "n".
pub fn ja_norm(s: &str) -> String {
    let mut t: String = s.chars().filter(|c| c.is_ascii_alphabetic()).map(|c| c.to_ascii_lowercase()).collect();
    for (a, b) in [("tsu", "tu"), ("shi", "si"), ("chi", "ti"), ("sh", "sy"), ("ch", "ty"), ("ji", "zi"), ("j", "zy"), ("fu", "hu"), ("dzu", "zu"), ("du", "zu"), ("di", "zi")] {
        t = t.replace(a, b);
    }
    for (a, b) in [("ou", "o"), ("oo", "o"), ("uu", "u"), ("aa", "a"), ("ii", "i"), ("ee", "e"), ("mb", "nb"), ("mp", "np"), ("mm", "nm"), ("nn", "n")] {
        while t.contains(a) {
            t = t.replace(a, b);
        }
    }
    t
}

/// Voiced forms of a reading's first consonant (rendaku): "kawa" -> "gawa", "hashi" -> "basi".
fn voiced(r: &str) -> Option<String> {
    let (a, rest) = r.split_at(1);
    let v = match a {
        "k" => "g",
        "s" => "z",
        "t" => "d",
        "h" => "b",
        _ => return None,
    };
    Some(format!("{}{}", v, rest))
}

fn kana_romaji(c: char) -> Option<&'static str> {
    // hiragana and katakana share the table by offset
    let c = if ('\u{30A1}'..='\u{30F6}').contains(&c) { char::from_u32(c as u32 - 0x60).unwrap() } else { c };
    const T: &[(char, &str)] = &[
        ('あ', "a"), ('い', "i"), ('う', "u"), ('え', "e"), ('お', "o"), ('か', "ka"), ('き', "ki"), ('く', "ku"), ('け', "ke"), ('こ', "ko"),
        ('が', "ga"), ('ぎ', "gi"), ('ぐ', "gu"), ('げ', "ge"), ('ご', "go"), ('さ', "sa"), ('し', "si"), ('す', "su"), ('せ', "se"), ('そ', "so"),
        ('ざ', "za"), ('じ', "zi"), ('ず', "zu"), ('ぜ', "ze"), ('ぞ', "zo"), ('た', "ta"), ('ち', "ti"), ('つ', "tu"), ('て', "te"), ('と', "to"),
        ('だ', "da"), ('ぢ', "zi"), ('づ', "zu"), ('で', "de"), ('ど', "do"), ('な', "na"), ('に', "ni"), ('ぬ', "nu"), ('ね', "ne"), ('の', "no"),
        ('は', "ha"), ('ひ', "hi"), ('ふ', "hu"), ('へ', "he"), ('ほ', "ho"), ('ば', "ba"), ('び', "bi"), ('ぶ', "bu"), ('べ', "be"), ('ぼ', "bo"),
        ('ぱ', "pa"), ('ぴ', "pi"), ('ぷ', "pu"), ('ぺ', "pe"), ('ぽ', "po"), ('ま', "ma"), ('み', "mi"), ('む', "mu"), ('め', "me"), ('も', "mo"),
        ('や', "ya"), ('ゆ', "yu"), ('よ', "yo"), ('ら', "ra"), ('り', "ri"), ('る', "ru"), ('れ', "re"), ('ろ', "ro"), ('わ', "wa"), ('ゐ', "i"),
        ('ゑ', "e"), ('を', "o"), ('ん', "n"), ('ゔ', "bu"), ('ぁ', "a"), ('ぃ', "i"), ('ぅ', "u"), ('ぇ', "e"), ('ぉ', "o"),
    ];
    T.iter().find(|(k, _)| *k == c).map(|(_, v)| *v)
}

/// Kana text to Kunrei romaji ("キョウト" -> "kyoto" after normalization), None if any other char.
fn kana_to_romaji(s: &str) -> Option<String> {
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut double = false;
    for (i, &c) in cs.iter().enumerate() {
        let c2 = if ('\u{30A1}'..='\u{30F6}').contains(&c) { char::from_u32(c as u32 - 0x60).unwrap() } else { c };
        match c2 {
            'っ' => {
                double = true;
                continue;
            }
            'ー' => continue,
            'ゃ' | 'ゅ' | 'ょ' => {
                // "ki" + small "ya" -> "kya"
                if out.ends_with('i') {
                    out.pop();
                }
                out.push('y');
                out.push(match c2 {
                    'ゃ' => 'a',
                    'ゅ' => 'u',
                    _ => 'o',
                });
                continue;
            }
            _ => {}
        }
        let r = kana_romaji(c2)?;
        if double {
            if let Some(f) = r.chars().next() {
                out.push(f);
            }
            double = false;
        }
        out.push_str(r);
        let _ = i;
    }
    Some(out)
}

fn is_han(c: char) -> bool {
    matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF) || c == '々' || c == '〆'
}

fn is_kana(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF)
}

/// The text is Han and Kana only (and at least one Han or Kana character).
pub fn readable(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| is_han(c) || is_kana(c) || c == 'ヶ' || c == 'ケ')
}

/// Does the romanized `latin` (ASCII letters) read as `native` (Han and Kana) in `lang`?
pub fn reads_as(native: &str, latin: &str, lang: Lang) -> bool {
    if !readable(native) || latin.is_empty() || !latin.bytes().all(|b| b.is_ascii_lowercase()) {
        return false;
    }
    let r = readings();
    // options per unit: a Han character, or a run of kana
    let mut units: Vec<Vec<String>> = Vec::new();
    let cs: Vec<char> = native.chars().collect();
    let mut i = 0;
    let mut prev: Option<char> = None;
    while i < cs.len() {
        let c = cs[i];
        if is_kana(c) && !(lang == Lang::Japanese && matches!(c, 'ヶ' | 'ケ') && i > 0 && is_han(cs[i - 1]) && i + 1 < cs.len() && is_han(cs[i + 1])) {
            let mut j = i;
            while j < cs.len() && is_kana(cs[j]) && !matches!(cs[j], 'ヶ') {
                j += 1;
            }
            if j == i {
                j = i + 1;
            }
            let run: String = cs[i..j].iter().collect();
            let rom = kana_to_romaji(&run).map(|x| ja_norm(&x));
            if lang != Lang::Japanese {
                return false;
            }
            units.push(rom.into_iter().collect());
            i = j;
            prev = None;
            continue;
        }
        let c = if c == '々' { prev.unwrap_or(c) } else { c };
        let opts: Vec<String> = match lang {
            Lang::Chinese => r.zh.get(&c).cloned().unwrap_or_default(),
            Lang::Japanese => {
                if matches!(c, 'ヶ' | 'ケ') {
                    vec!["ga".into(), "ke".into(), "ka".into()]
                } else {
                    r.ja.get(&c).cloned().unwrap_or_default()
                }
            }
        };
        if opts.is_empty() {
            return false;
        }
        units.push(opts);
        prev = Some(c);
        i += 1;
    }
    let lat = match lang {
        Lang::Japanese => ja_norm(latin),
        Lang::Chinese => latin.to_string(),
    };
    let lb = lat.as_bytes();
    let n = units.len();
    // reach[k] = set of positions in `lat` after matching units[..k]
    let mut reach: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
    reach[0].push(0);
    for k in 0..n {
        let from = std::mem::take(&mut reach[k]);
        for &p in &from {
            for o in &units[k] {
                let mut forms: Vec<String> = vec![o.clone()];
                if lang == Lang::Japanese && k > 0 {
                    if let Some(v) = voiced(o) {
                        forms.push(v);
                    }
                    if o.starts_with('h') {
                        forms.push(format!("p{}", &o[1..]));
                    }
                }
                for f in &forms {
                    if lb.len() >= p + f.len() && &lat[p..p + f.len()] == f.as_str() {
                        let q = p + f.len();
                        if !reach[k + 1].contains(&q) {
                            reach[k + 1].push(q);
                        }
                    }
                    // a doubled consonant: "tu" / "ti" / "ki" / "ku" ending a reading before a
                    // consonant ("satu" + "horo" -> "sapporo")
                    if lang == Lang::Japanese && k + 1 < n && f.len() >= 3 && (f.ends_with("tu") || f.ends_with("ti") || f.ends_with("ki") || f.ends_with("ku")) {
                        let stem = &f[..f.len() - 2];
                        let q = p + stem.len();
                        if lb.len() > q + 1 && &lat[p..q] == stem && !b"aeioun".contains(&lb[q]) {
                            // the next unit must start with the doubled consonant
                            let q2 = q + 1;
                            if lb[q2..].first() == Some(&lb[q]) || (lb[q] == b't' && lb.get(q2) == Some(&b'y')) {
                                if !reach[k + 1].contains(&q2) {
                                    reach[k + 1].push(q2);
                                }
                            }
                        }
                    }
                }
            }
        }
        if reach[k + 1].is_empty() {
            return false;
        }
    }
    reach[n].contains(&lb.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chinese_spellings() {
        assert!(reads_as("台中", "taichung", Lang::Chinese));
        assert!(reads_as("台中", "taizhong", Lang::Chinese));
        assert!(reads_as("高雄", "kaohsiung", Lang::Chinese));
        assert!(reads_as("錦州", "jinzhou", Lang::Chinese));
        assert!(reads_as("中山", "jhongshan", Lang::Chinese));
        assert!(reads_as("朝阳", "chaoyang", Lang::Chinese));
        assert!(!reads_as("錦州", "jingcheng", Lang::Chinese));
        assert!(!reads_as("台中", "tainan", Lang::Chinese));
    }
    #[test]
    fn japanese_readings() {
        assert!(reads_as("久留米", "kurume", Lang::Japanese));
        assert!(reads_as("旭町", "asahicho", Lang::Japanese));
        assert!(reads_as("右京", "ukyo", Lang::Japanese));
        assert!(reads_as("京都", "kyoto", Lang::Japanese));
        assert!(reads_as("長岡", "nagaoka", Lang::Japanese));
        assert!(!reads_as("長岡", "nagano", Lang::Japanese));
        assert!(!reads_as("旭町", "sakuragicho", Lang::Japanese));
    }
}
