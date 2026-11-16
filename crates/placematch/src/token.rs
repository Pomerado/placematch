//! Segmentation and tokenization of folded text, including CJK block-number forms.
use crate::fold::{cjk_numerals, script_of, Script};
use regex_lite::Regex;

#[derive(Clone, Debug)]
pub struct Tok {
    /// folded surface text (native script)
    pub s: String,
    pub seg: usize,
    pub script: Script,
    pub num: bool,
    /// written as an abbreviation (followed by a dot)
    pub abbr: bool,
}

pub struct CjkRules {
    rules: Vec<(Regex, &'static str)>,
    kr_ho: Regex,
}

impl CjkRules {
    pub fn new() -> CjkRules {
        let r = |p: &str| Regex::new(p).unwrap();
        CjkRules {
            rules: vec![
                (r(r"(\d+(?:-\d+)+)\s*(?:番地|番|号|號)"), " $1 "),
                (r(r"(\d+)\s*丁目\s*(\d+)\s*番地?\s*の?\s*(\d+)\s*号?"), " $1-$2-$3 "),
                (r(r"(\d+)\s*丁目\s*(\d+)\s*[-‐−の]\s*(\d+)\s*号?"), " $1-$2-$3 "),
                (r(r"(\d+)\s*丁目\s*(\d+)\s*番地?"), " $1-$2 "),
                (r(r"(\d+)\s*丁目\s*(\d+)\b"), " $1-$2 "),
                (r(r"(\d+)\s*番地?\s*の?\s*(\d+)\s*号"), " $1-$2 "),
                (r(r"(\d+)\s*丁目"), " $1 "),
                (r(r"(\d+)\s*(?:番地|番)"), " $1 "),
                // Taiwanese sub-numbers: "6之6號" is house 6-6, not house 6
                (r(r"(\d+)\s*之\s*(\d+)\s*(?:号|號)"), " no $1-$2 "),
                (r(r"(\d+)\s*(?:号室|號室|室|호)"), " room $1 "),
                (r(r"(\d+)\s*(?:階|楼|層|层|층)"), " floor $1 "),
                (r(r"(\d+)\s*(?:号|號|번지|번)"), " no $1 "),
                (r(r"(\d+)\s*-?\s*chome\s*,?\s*(\d+)\s*-?\s*ban(?:chi)?\s*,?\s*(\d+)\s*-?\s*go\b"), " $1-$2-$3 "),
                // "3 Shonai-cho 6-chome": the lot number written before the town and its chome
                (r(r"\b(\d+)\s+([a-z][a-z\- ]*?)\s+(\d+)\s*-?\s*chome\b"), " $2 $3-$1 "),
                (r(r"(\d+)\s*-?\s*chome\s*,?\s*(\d+)\s*-?\s*ban(?:chi)?\b"), " $1-$2 "),
                (r(r"\bchome\s*(\d+)\b"), " $1 "),
                (r(r"(\d+)\s*-?\s*chome\b"), " $1 "),
                (r(r"〒"), " "),
            ],
            kr_ho: r(r"\b(\d+)\s*-?\s*ho\b"),
        }
    }

    /// Romanized Korean unit numbers ("20ho") become unit keywords.
    pub fn korean_romanized(&self, s: &str) -> String {
        if !s.contains("ho") {
            return s.to_string();
        }
        self.kr_ho.replace_all(s, " room $1 ").into_owned()
    }

    pub fn apply(&self, s: &str) -> String {
        let has_marker = s.chars().any(|c| !c.is_ascii() && matches!(script_of(c), Script::Han | Script::Kana | Script::Hangul)) || s.contains("chome");
        if !has_marker {
            return s.to_string();
        }
        let mut t = cjk_numerals(s);
        for (re, rep) in &self.rules {
            if re.is_match(&t) {
                t = re.replace_all(&t, *rep).into_owned();
            }
        }
        t
    }
}

fn is_seg_sep(c: char) -> bool {
    matches!(c, ',' | ';' | '\n' | '،' | '、' | '|' | '(' | ')' | '[' | ']' | '؛' | '·' | '•')
}

/// Japanese: split after municipality suffixes only (道/街 occur inside city names).
fn jp_split_after(c: char, cur: &str) -> bool {
    matches!(c, '都' | '府' | '県' | '市' | '区' | '郡' | '町' | '村') || cur == "北海道"
}

/// Characters after which a CJK run is split (admin and street suffixes).
fn cjk_split_after(c: char) -> bool {
    matches!(
        c,
        '都' | '道' | '府' | '県' | '縣' | '省' | '市' | '区' | '區' | '郡' | '町' | '村' | '县' | '镇' | '鎮' | '乡' | '鄉' | '路' | '街' | '巷' | '弄' | '里'
    )
}

const CJK_COUNTRY_PREFIXES: &[&str] = &["中华人民共和国", "中華民國", "中国", "中國", "臺灣", "台灣", "台湾", "日本国", "日本"];

/// Split a leading country name off a CJK chunk ("臺灣高雄市" -> "臺灣", "高雄市").
fn split_country_prefix(toks: Vec<Tok>) -> Vec<Tok> {
    let mut out = Vec::with_capacity(toks.len());
    for t in toks {
        if t.script.is_cjk() {
            if let Some(p) = CJK_COUNTRY_PREFIXES.iter().find(|p| t.s.starts_with(**p) && t.s.len() > p.len()) {
                let rest = t.s[p.len()..].to_string();
                out.push(Tok { s: p.to_string(), ..t.clone() });
                out.push(Tok { s: rest, ..t });
                continue;
            }
        }
        out.push(t);
    }
    out
}

/// Tokenize folded text into tokens with segment numbers.
pub fn tokenize(folded: &str, jp: bool) -> Vec<Tok> {
    split_country_prefix(tokenize_raw(folded, jp))
}

fn tokenize_raw(folded: &str, jp: bool) -> Vec<Tok> {
    let chars: Vec<char> = folded.chars().collect();
    let mut toks: Vec<Tok> = Vec::new();
    let mut seg = 0usize;
    let mut cur = String::new();
    let mut cur_sc = Script::Other;
    let flush = |cur: &mut String, sc: Script, seg: usize, toks: &mut Vec<Tok>| {
        if !cur.is_empty() {
            let num = cur.chars().next().map_or(false, |c| c.is_ascii_digit());
            toks.push(Tok { s: std::mem::take(cur), seg, script: if num { Script::Digit } else { sc }, num, abbr: false });
        }
    };
    let n = chars.len();
    for i in 0..n {
        let c = chars[i];
        if is_seg_sep(c) {
            flush(&mut cur, cur_sc, seg, &mut toks);
            if toks.last().map_or(false, |t| t.seg == seg) {
                seg += 1;
            }
            continue;
        }
        if c == '#' {
            flush(&mut cur, cur_sc, seg, &mut toks);
            toks.push(Tok { s: "#".into(), seg, script: Script::Other, num: false, abbr: false });
            continue;
        }
        if (c == '-' || c == '/' || c == '‐' || c == '−') && !cur.is_empty() {
            let prev_digit = cur.chars().last().map_or(false, |p| p.is_ascii_digit());
            let next_digit = chars.get(i + 1).map_or(false, |p| p.is_ascii_digit());
            if (prev_digit || c == '/') && next_digit && cur.chars().next().map_or(false, |p| p.is_ascii_digit()) {
                cur.push(if c == '/' { '/' } else { '-' });
                continue;
            }
        }
        if c.is_alphanumeric() || (crate::fold::script_of(c) != Script::Other && !c.is_whitespace() && !c.is_ascii_punctuation()) {
            let sc = script_of(c);
            if !cur.is_empty() {
                let last = cur.chars().last().unwrap();
                let lsc = script_of(last);
                // split between CJK and anything else, and between digits and CJK
                let cjk_now = matches!(sc, Script::Han | Script::Kana);
                let cjk_last = matches!(lsc, Script::Han | Script::Kana);
                let boundary = (cjk_now != cjk_last)
                    || (cjk_last && if jp { jp_split_after(last, &cur) } else { cjk_split_after(last) })
                    || (lsc == Script::Digit && sc != Script::Digit && sc != Script::Latin && !(matches!(sc, Script::Cyrillic | Script::Greek) && !chars.get(i + 1).map_or(false, |n| n.is_alphabetic())))
                    || (sc == Script::Digit && lsc != Script::Digit && lsc != Script::Latin && !cur.chars().next().map_or(false, |p| p.is_ascii_digit()))
                    || (sc == Script::Digit && lsc == Script::Latin && !cur.chars().next().map_or(false, |p| p.is_ascii_digit()))
                    || (lsc == Script::Hangul && sc == Script::Latin)
                    || (lsc == Script::Latin && sc == Script::Hangul);
                if boundary {
                    flush(&mut cur, cur_sc, seg, &mut toks);
                }
            }
            if sc != Script::Digit && sc != Script::Other {
                cur_sc = sc;
            } else if cur.is_empty() {
                cur_sc = sc;
            }
            cur.push(c);
        } else if matches!(c, '.' | '\'' | '’') {
            let had = !cur.is_empty();
            flush(&mut cur, cur_sc, seg, &mut toks);
            if had {
                if let Some(t) = toks.last_mut() {
                    t.abbr = true;
                }
            }
        } else {
            flush(&mut cur, cur_sc, seg, &mut toks);
        }
    }
    flush(&mut cur, cur_sc, seg, &mut toks);
    toks
}

/// Tokens of a dictionary phrase (no segments).
pub fn tokenize_plain(folded: &str) -> Vec<String> {
    tokenize(folded, false).into_iter().map(|t| t.s).collect()
}
