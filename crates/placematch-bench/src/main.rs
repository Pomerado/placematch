//! placematch-bench: predictions (default) or latency mode (--latency) over benchmark JSONL on stdin.
use placematch::{Matcher, Pick, Verdict};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::time::Instant;

fn predict(m: &Matcher, case: &Value) -> Value {
    let id = case["id"].clone();
    let q = case["query"].as_str().unwrap_or("");
    let cc = case["country"].as_str();
    let cands: Vec<&str> = case["candidates"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
    if case["task"] == "pair" {
        let (v, _) = m.compare(q, cands.first().copied().unwrap_or(""), cc);
        return if v == Verdict::Same { json!({"id": id, "outcome": "match", "index": 0}) } else { json!({"id": id, "outcome": "none", "index": null}) };
    }
    match m.pick(q, &cands, cc) {
        Pick::Match(i) => json!({"id": id, "outcome": "match", "index": i}),
        Pick::None => json!({"id": id, "outcome": "none", "index": null}),
        Pick::Ambiguous(_) => json!({"id": id, "outcome": "ambiguous", "index": null}),
    }
}

fn pct(v: &mut Vec<f64>, p: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if v.is_empty() { return 0.0; }
    v[((v.len() as f64 - 1.0) * p).round() as usize]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let t0 = Instant::now();
    let m = Matcher::new();
    let init = t0.elapsed();
    let cases: Vec<Value> = std::io::stdin().lock().lines().filter_map(|l| l.ok()).filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(&l).unwrap()).collect();
    if args.iter().any(|a| a == "--explain") {
        for c in &cases {
            let q = c["query"].as_str().unwrap_or("");
            let cc = c["country"].as_str();
            println!("{} [{}] Q: {}\n   {:?}", c["id"], cc.unwrap_or("-"), q, m.normalize(q, cc));
            for (i, cand) in c["candidates"].as_array().unwrap().iter().enumerate() {
                let s = cand.as_str().unwrap();
                let (v, r) = m.compare(q, s, cc);
                println!("  {} {:?} {:?} :: {}\n     {:?}", i, v, r, s, m.normalize(s, cc));
            }
        }
        return;
    }
    if args.iter().any(|a| a == "--stages") {
        let texts: Vec<(String, Option<String>)> = cases.iter().flat_map(|c| {
            let cc = c["country"].as_str().map(|s| s.to_string());
            let mut v = vec![(c["query"].as_str().unwrap_or("").to_string(), cc.clone())];
            for x in c["candidates"].as_array().unwrap() { v.push((x.as_str().unwrap().to_string(), cc.clone())); }
            v
        }).collect();
        let t = Instant::now();
        let mut acc = 0usize;
        for (s, _) in &texts { acc += placematch::fold::fold(s).len(); }
        let fold_t = t.elapsed();
        let t = Instant::now();
        for (s, _) in &texts { acc += placematch::token::tokenize(&placematch::fold::fold(s), false).len(); }
        let tok_t = t.elapsed();
        let t = Instant::now();
        for (s, cc) in &texts { acc += m.normalize(s, cc.as_deref()).units.len(); }
        let parse_t = t.elapsed();
        let n = texts.len() as f64;
        eprintln!("texts={} fold_us={:.2} fold+tokenize_us={:.2} parse_us={:.2} ({})", texts.len(), fold_t.as_secs_f64()*1e6/n, tok_t.as_secs_f64()*1e6/n, parse_t.as_secs_f64()*1e6/n, acc);
        return;
    }
    if args.iter().any(|a| a == "--latency") {
        let rounds: usize = 3;
        let mut pick_t = Vec::new();
        let mut cmp_t = Vec::new();
        let tall = Instant::now();
        let mut calls = 0usize;
        for _ in 0..rounds {
            for c in &cases {
                let q = c["query"].as_str().unwrap_or("");
                let cc = c["country"].as_str();
                let cands: Vec<&str> = c["candidates"].as_array().unwrap().iter().filter_map(|x| x.as_str()).collect();
                let t = Instant::now();
                let r = m.pick(q, &cands, cc);
                pick_t.push(t.elapsed().as_secs_f64() * 1e6);
                std::hint::black_box(r);
                let t = Instant::now();
                let r = m.compare(q, cands[0], cc);
                cmp_t.push(t.elapsed().as_secs_f64() * 1e6);
                std::hint::black_box(r);
                calls += 1;
            }
        }
        let total = tall.elapsed().as_secs_f64();
        let mut p = pick_t.clone();
        let mut cm = cmp_t.clone();
        eprintln!("init_ms={:.2} cases={} rounds={} pick_p50_us={:.1} pick_p99_us={:.1} compare_p50_us={:.1} compare_p99_us={:.1} pick+compare_per_s_one_core={:.0}",
            init.as_secs_f64() * 1e3, cases.len(), rounds, pct(&mut p, 0.5), pct(&mut p, 0.99), pct(&mut cm, 0.5), pct(&mut cm, 0.99), calls as f64 / total);
        let picks_per_s = pick_t.len() as f64 / (pick_t.iter().sum::<f64>() / 1e6);
        let cmps_per_s = cmp_t.len() as f64 / (cmp_t.iter().sum::<f64>() / 1e6);
        eprintln!("pick_calls_per_s_one_core={:.0} compare_calls_per_s_one_core={:.0}", picks_per_s, cmps_per_s);
        return;
    }
    let out = std::io::stdout();
    let mut w = std::io::BufWriter::new(out.lock());
    for c in &cases {
        writeln!(w, "{}", predict(&m, c)).unwrap();
    }
    eprintln!("init_ms={:.2} cases={} total_ms={:.1}", init.as_secs_f64() * 1e3, cases.len(), t0.elapsed().as_secs_f64() * 1e3);
}
