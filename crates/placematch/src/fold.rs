//! Character folding, script detection and transliteration.
use unicode_normalization::char::{decompose_canonical, is_combining_mark};
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Script {
    Latin,
    Greek,
    Cyrillic,
    Hebrew,
    Arabic,
    Devanagari,
    Thai,
    Hangul,
    Kana,
    Han,
    Other,
    Digit,
}

impl Script {
    pub fn is_cjk(self) -> bool {
        matches!(self, Script::Han | Script::Kana)
    }
}

pub fn script_of(c: char) -> Script {
    let u = c as u32;
    if c.is_ascii_digit() {
        return Script::Digit;
    }
    match u {
        0x41..=0x5A | 0x61..=0x7A | 0xC0..=0x24F | 0x1E00..=0x1EFF => Script::Latin,
        0x370..=0x3FF | 0x1F00..=0x1FFF => Script::Greek,
        0x400..=0x52F => Script::Cyrillic,
        0x590..=0x5FF => Script::Hebrew,
        0x600..=0x6FF | 0x750..=0x77F | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => Script::Arabic,
        0x900..=0x97F => Script::Devanagari,
        0xE00..=0xE7F => Script::Thai,
        0xAC00..=0xD7AF | 0x1100..=0x11FF | 0x3130..=0x318F => Script::Hangul,
        0x3040..=0x30FF | 0x31F0..=0x31FF => Script::Kana,
        0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF => Script::Han,
        _ => Script::Other,
    }
}

/// Zero code points of decimal digit blocks (Unicode Nd), ASCII and full-width handled by NFKC.
const DIGIT_ZEROS: &[u32] = &[
    0x660, 0x6F0, 0x7C0, 0x966, 0x9E6, 0xA66, 0xAE6, 0xB66, 0xBE6, 0xC66, 0xCE6, 0xD66, 0xDE6, 0xE50,
    0xED0, 0xF20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0, 0x1A80, 0x1A90, 0x1B50, 0x1BB0,
    0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0, 0xAA50, 0xABF0, 0xFF10,
];

pub fn native_digit(c: char) -> Option<char> {
    let u = c as u32;
    for &z in DIGIT_ZEROS {
        if u >= z && u < z + 10 {
            return char::from_digit(u - z, 10);
        }
    }
    None
}

fn kanji_digit(c: char) -> Option<u32> {
    Some(match c {
        '〇' | '零' => 0,
        '一' => 1,
        '二' => 2,
        '三' => 3,
        '四' => 4,
        '五' => 5,
        '六' => 6,
        '七' => 7,
        '八' => 8,
        '九' => 9,
        _ => return None,
    })
}

/// Convert runs of CJK numerals (一 .. 九, 十, 百) that precede a CJK address marker into ASCII digits.
pub fn cjk_numerals(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        while i < chars.len() && (kanji_digit(chars[i]).is_some() || chars[i] == '十' || chars[i] == '百') {
            i += 1;
        }
        if i > start {
            let next = chars.get(i).copied();
            let marker = matches!(next, Some('丁' | '番' | '号' | '號' | '条' | '階' | '楼' | '层' | '室' | '弄' | '巷'));
            if marker {
                out.push_str(&parse_kanji_number(&chars[start..i]).to_string());
            } else {
                out.extend(&chars[start..i]);
            }
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn parse_kanji_number(cs: &[char]) -> u32 {
    let mut total = 0;
    let mut cur = 0;
    let mut positional = true;
    for &c in cs {
        if c == '十' || c == '百' {
            positional = false;
        }
    }
    if positional {
        for &c in cs {
            cur = cur * 10 + kanji_digit(c).unwrap_or(0);
        }
        return cur;
    }
    for &c in cs {
        match c {
            '十' => {
                total += if cur == 0 { 1 } else { cur } * 10;
                cur = 0;
            }
            '百' => {
                total += if cur == 0 { 1 } else { cur } * 100;
                cur = 0;
            }
            _ => cur = kanji_digit(c).unwrap_or(0),
        }
    }
    total + cur
}

/// Fold text: NFKC, lowercase, native digits to ASCII, Latin and Greek diacritics removed,
/// special letters expanded, Arabic and Hebrew vowel marks removed, apostrophes dropped.
pub fn fold(s: &str) -> String {
    if s.is_ascii() && !s.contains(['\'', '`']) {
        return s.to_ascii_lowercase();
    }
    let mut out = String::with_capacity(s.len() + 8);
    let cs: Vec<char> = s.nfkc().collect();
    for (i, &ch) in cs.iter().enumerate() {
        // an apostrophe or geresh ending a word marks an abbreviation ("רח'"): keep it as a dot
        if matches!(ch, '\'' | '’' | '׳' | '״') {
            let next = cs.get(i + 1).copied();
            if next.map_or(true, |n| n.is_whitespace() || n.is_ascii_punctuation() || n == '،') && i > 0 && cs[i - 1].is_alphabetic() {
                out.push('.');
            }
            continue;
        }
        if let Some(d) = native_digit(ch) {
            out.push(d);
            continue;
        }
        for lc in ch.to_lowercase() {
            push_folded(lc, &mut out);
        }
    }
    out
}

fn push_folded(c: char, out: &mut String) {
    match c {
        'ß' | 'ẞ' => out.push_str("ss"),
        'ł' => out.push('l'),
        'đ' | 'ð' => out.push('d'),
        'ø' => out.push('o'),
        'æ' => out.push_str("ae"),
        'œ' => out.push_str("oe"),
        'ı' => out.push('i'),
        'þ' => out.push_str("th"),
        'ħ' => out.push('h'),
        'ŧ' => out.push('t'),
        '\'' | '’' | '‘' | '`' | 'ʼ' | 'ʻ' | '´' | 'ʹ' => {}
        'ـ' => {} // Arabic tatweel
        'ё' => out.push('е'),
        'ς' => out.push('σ'),
        _ => {
            let sc = script_of(c);
            if matches!(sc, Script::Latin | Script::Greek) && !c.is_ascii() {
                decompose_canonical(c, |d| {
                    if !is_combining_mark(d) {
                        out.push(d)
                    }
                });
            } else if matches!(sc, Script::Arabic | Script::Hebrew) && is_combining_mark(c) {
            } else if c as u32 == 0x670 {
            } else {
                out.push(c)
            }
        }
    }
}

/// ASCII key of a folded token: transliterates non-Latin scripts, keeps [a-z0-9].
pub fn ascii_key(tok: &str) -> String {
    let mut s = String::with_capacity(tok.len());
    if tok.is_ascii() {
        s.extend(tok.chars().filter(|c| c.is_ascii_alphanumeric()));
    } else {
        let t = any_ascii::any_ascii(tok);
        s.extend(t.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()));
    }
    umlaut_fold(&s)
}

/// "muenchen" and "munchen" fold to the same key; applied on both sides.
fn umlaut_fold(s: &str) -> String {
    if !(s.contains("ae") || s.contains("oe") || s.contains("ue")) {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if i + 1 < b.len() && b[i + 1] == b'e' && matches!(b[i], b'a' | b'o' | b'u') && i > 0 && !matches!(b[i - 1], b'a' | b'e' | b'i' | b'o' | b'u' | b'q') {
            out.push(b[i] as char);
            i += 2;
            continue;
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

/// Consonant key for names written in Hebrew or Arabic letters and their romanizations: these
/// scripts leave most vowels out and romanizations spell one letter several ways ("kh" / "ch" /
/// "k", "f" / "p", "ts" / "tz" / "z"). Letters that stand for vowels as often as consonants (y, w,
/// and in Hebrew v and b) are dropped on both sides. `hebrew`: use the Hebrew rules.
pub fn abjad_key(s: &str, hebrew: bool) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut last = 0u8;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let n = b.get(i + 1).copied().unwrap_or(0);
        let (mut c, adv) = match (c, n) {
            (b'k', b'h') | (b'c', b'h') | (b'c', b'k') => (b'k', 2),
            (b's', b'h') | (b't', b's') | (b't', b'z') | (b'z', b'h') => (b's', 2),
            (b't', b'h') => (b't', 2),
            (b'd', b'h') => (b'd', 2),
            (b'g', b'h') => (b'g', 2),
            (b'p', b'h') => (b'p', 2),
            _ => (c, 1),
        };
        i += adv;
        c = match c {
            b'q' | b'c' | b'x' => b'k',
            b'f' => b'p',
            b'z' => b's',
            b'j' => b'g',
            _ => c,
        };
        let drop = matches!(c, b'a' | b'e' | b'i' | b'o' | b'u' | b'y' | b'w' | b'h' | b'\'') || (hebrew && matches!(c, b'v' | b'b'));
        if drop || c == last || !c.is_ascii_alphanumeric() {
            continue;
        }
        out.push(c as char);
        last = c;
    }
    out
}

/// Consonant skeleton for comparing a transliterated name with a romanized one.
pub fn skeleton(s: &str) -> String {
    // digraphs first (kh->h, sh->s, ch->c, zh->z, ts/tz->c, th->t, dh->d, gh->g, ph->f, ck->k),
    // then drop vowels, h and y, merge q/k, w/v, x/k, j/y, and collapse repeats
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut last = 0u8;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let n = b.get(i + 1).copied().unwrap_or(0);
        let (mut c, adv) = match (c, n) {
            (b'k', b'h') => (b'h', 2),
            (b's', b'h') => (b's', 2),
            (b'c', b'h') => (b'c', 2),
            (b'z', b'h') => (b'z', 2),
            (b't', b's') | (b't', b'z') => (b'c', 2),
            (b't', b'h') => (b't', 2),
            (b'd', b'h') => (b'd', 2),
            (b'g', b'h') => (b'g', 2),
            (b'p', b'h') => (b'f', 2),
            (b'c', b'k') => (b'k', 2),
            _ => (c, 1),
        };
        i += adv;
        c = match c {
            b'q' | b'x' => b'k',
            b'w' => b'v',
            b'j' => b'y',
            _ => c,
        };
        if matches!(c, b'a' | b'e' | b'i' | b'o' | b'u' | b'y' | b'h' | b'\'') || c == last {
            continue;
        }
        out.push(c as char);
        last = c;
    }
    out
}

pub fn dominant_script(s: &str) -> Script {
    let mut counts = [0usize; 12];
    for c in s.chars() {
        let sc = script_of(c);
        if !matches!(sc, Script::Other | Script::Digit) {
            counts[sc as usize] += 1;
        }
    }
    let (i, n) = counts.iter().enumerate().max_by_key(|(_, n)| **n).unwrap();
    if *n == 0 {
        return Script::Latin;
    }
    [
        Script::Latin,
        Script::Greek,
        Script::Cyrillic,
        Script::Hebrew,
        Script::Arabic,
        Script::Devanagari,
        Script::Thai,
        Script::Hangul,
        Script::Kana,
        Script::Han,
        Script::Other,
        Script::Digit,
    ][i]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folds() {
        assert_eq!(fold("München"), "munchen");
        assert_eq!(ascii_key(&fold("Muenchen")), "munchen");
        assert_eq!(fold("١٢٣"), "123");
        assert_eq!(fold("१२३"), "123");
        assert_eq!(fold("１２３ＡＢ"), "123ab");
        assert_eq!(fold("۱۲"), "12");
        assert_eq!(fold("๑๒"), "12");
        assert_eq!(cjk_numerals("三丁目十二番"), "3丁目12番");
    }
}
