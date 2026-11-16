//! Hand-written fictional cases (tests/cases.jsonl): every one must get its expected outcome.
use placematch::{Matcher, Pick, Verdict};
use serde_json::Value;

#[test]
fn fixture_cases() {
    let m = Matcher::new();
    let text = include_str!("../../../tests/cases.jsonl");
    let mut failures = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let c: Value = serde_json::from_str(line).unwrap();
        let q = c["query"].as_str().unwrap();
        let cc = c["country"].as_str();
        let cands: Vec<&str> = c["candidates"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
        let accept: Vec<usize> = c["accept"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as usize).collect();
        let expected = c["expected"].as_str().unwrap();
        let ok = if c["task"] == "pair" {
            let same = m.compare(q, cands[0], cc).0 == Verdict::Same;
            same == (expected == "match")
        } else {
            match m.pick(q, &cands, cc) {
                Pick::Match(i) => expected == "match" && accept.contains(&i),
                Pick::None => expected == "none",
                Pick::Ambiguous(_) => expected == "ambiguous",
            }
        };
        if !ok {
            failures.push(c["id"].as_str().unwrap().to_string());
        }
    }
    assert!(failures.is_empty(), "failed fixture cases: {:?}", failures);
}

#[test]
fn pick_is_order_invariant() {
    let m = Matcher::new();
    let a = ["12 Maple Ave, Springfield, IL 62704", "120 Maple Ave, Springfield, IL 62704"];
    let b = [a[1], a[0]];
    assert_eq!(m.pick("12 Maple Avenue, Springfield", &a, Some("US")), Pick::Match(0));
    assert_eq!(m.pick("12 Maple Avenue, Springfield", &b, Some("US")), Pick::Match(1));
}

#[test]
fn readme_example() {
    use placematch::{Matcher, Pick};
    let m = Matcher::new();
    let pick = m.pick(
        "12 Maple Ave Apt 4, Springfield IL",
        &["12 Maple Avenue, Apt 4, Springfield, IL 62704", "120 Maple Avenue, Springfield, IL 62704"],
        Some("US"),
    );
    assert_eq!(pick, Pick::Match(0));
}
