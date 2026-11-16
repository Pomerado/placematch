// Packs the vendored dictionaries and country data into one text blob that the
// library embeds with include_str!. Format, one record per line:
//   D <lang> <class> <pipe-separated synonyms>
//   C <CC> <langs comma> <zip regex> <names ~-separated>
use std::{env, fs, path::Path};

#[path = "src/fold.rs"]
#[allow(dead_code)]
mod fold;
#[path = "src/token.rs"]
#[allow(dead_code)]
mod token;
#[path = "src/data.rs"]
#[allow(dead_code)]
mod data;

fn main() {
    let data = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("data");
    println!("cargo:rerun-if-changed=data");
    let mut out = String::new();
    let mut langs: Vec<_> = fs::read_dir(data.join("dict")).unwrap().map(|e| e.unwrap().path()).collect();
    langs.sort();
    for dir in langs {
        let lang = dir.file_name().unwrap().to_string_lossy().to_string();
        let mut files: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
        files.sort();
        for f in files {
            let class = f.file_stem().unwrap().to_string_lossy().to_string();
            for line in fs::read_to_string(&f).unwrap().lines() {
                let line = line.trim();
                if line.is_empty() { continue; }
                out.push_str(&format!("D\t{}\t{}\t{}\n", lang, class, line));
            }
        }
    }
    // placematch supplements (same record shape)
    for line in fs::read_to_string(data.join("extra/units.txt")).unwrap().lines() {
        if line.starts_with('#') || line.trim().is_empty() { continue; }
        let f: Vec<&str> = line.splitn(3, '\t').collect();
        out.push_str(&format!("D\t{}\t{}\t{}\n", f[0], f[1], f[2]));
    }
    // Wikidata's names first, so the hand list wins where both name a city
    for line in fs::read_to_string(data.join("extra/exonyms-wikidata.txt")).unwrap().lines() {
        if line.starts_with('#') || line.trim().is_empty() { continue; }
        out.push_str(&format!("E\t{}\n", line.trim()));
    }
    for line in fs::read_to_string(data.join("extra/exonyms.txt")).unwrap().lines() {
        if line.starts_with('#') || line.trim().is_empty() { continue; }
        out.push_str(&format!("E\t{}\n", line.trim()));
    }
    // country -> languages (OpenCage), names of regions (OpenCage state codes + libaddressinput)
    let mut c2l = std::collections::BTreeMap::new();
    for line in fs::read_to_string(data.join("country2lang.yaml")).unwrap().lines() {
        if line.starts_with('#') || !line.contains(':') { continue; }
        let (k, v) = line.split_once(':').unwrap();
        c2l.insert(k.trim().to_string(), v.trim().to_string());
    }
    let mut regions: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    let mut cur = String::new();
    for line in fs::read_to_string(data.join("state_codes.yaml")).unwrap().lines() {
        if line.trim_start().starts_with('#') || line.trim().is_empty() { continue; }
        let indent = line.len() - line.trim_start().len();
        let t = line.trim();
        let (k, v) = match t.split_once(':') { Some(x) => x, None => continue };
        let v = v.trim().trim_matches('"').trim_matches('\'');
        if indent == 0 { cur = k.trim().to_string(); continue; }
        let e = regions.entry(cur.clone()).or_default();
        if indent <= 4 { e.push(k.trim().to_string()); }
        if !v.is_empty() { e.push(v.to_string()); }
    }
    let mut ccs = std::collections::BTreeSet::new();
    ccs.extend(c2l.keys().cloned());
    let mut zips = std::collections::BTreeMap::new();
    let mut cnames: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for e in fs::read_dir(data.join("aid")).unwrap() {
        let p = e.unwrap().path();
        let j: serde_json::Value = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
        let cc = j["key"].as_str().unwrap().to_string();
        if let Some(z) = j["zip"].as_str() { zips.insert(cc.clone(), z.to_string()); }
        // the English country name, without notes such as "(Rep.)"
        if let Some(n) = j["name"].as_str() { cnames.insert(cc.clone(), n.split('(').next().unwrap_or("").trim().to_string()); }
        let r = regions.entry(cc.clone()).or_default();
        for f in ["sub_keys", "sub_names", "sub_lnames", "sub_isoids"] {
            if let Some(s) = j[f].as_str() { r.extend(s.split('~').map(|x| x.to_string())); }
        }
        ccs.insert(cc);
    }
    for cc in ccs {
        let names: Vec<String> = regions.get(&cc).cloned().unwrap_or_default().into_iter().filter(|s| !s.is_empty() && !s.contains('\t')).collect();
        out.push_str(&format!("C\t{}\t{}\t{}\t{}\t{}\n", cc, c2l.get(&cc).cloned().unwrap_or_default(), zips.get(&cc).cloned().unwrap_or_default(), names.join("~"), cnames.get(&cc).cloned().unwrap_or_default()));
    }
    let dst = Path::new(&env::var("OUT_DIR").unwrap()).join("placematch-data.txt");
    let d = data::Data::build_from_raw(&out);
    fs::write(&dst, d.serialize()).unwrap();
}
