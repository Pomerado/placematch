//! Hand-written fictional cases (tests/cases.jsonl): every one must get its expected outcome.
use placematch::{Matcher, Outcome, Pick, PickOptions, Row, Verdict};
use serde_json::Value;

#[test]
fn fixture_cases() {
    run_cases(include_str!("../../../tests/cases.jsonl"));
}

/// Places named without a house number (tests/place-names.jsonl): the core answer for each, with
/// the picker's answer for its mode alongside (a picker asks where PlaceMatch says none and rows
/// were offered, and a read may take an in-area row).
#[test]
fn place_name_cases() {
    run_cases(include_str!("../../../tests/place-names.jsonl"));
}

fn run_cases(text: &str) {
    let m = Matcher::new();
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
        // a case may also name the notes its answer carries ("unit_not_listed")
        let notes_ok = c.get("notes").map_or(true, |want| {
            let rows: Vec<Row> = cands.iter().map(|&t| Row::from(t)).collect();
            let got = m.pick_explained(q, &rows, &PickOptions { country: cc }).notes;
            want.as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect::<Vec<_>>() == got
        });
        if !ok || !notes_ok {
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

/// The picker built on PlaceMatch, as the place-name cases describe it: a write asks when PlaceMatch
/// finds no place among usable rows; a read takes the row PlaceMatch prefers on a read (the
/// categorized venue beside a same-name pin), else, for a named place, the site's first usable row
/// when it is in the caller's area, and otherwise refuses. Controls are never usable.
#[test]
fn place_name_picker_modes() {
    let m = Matcher::new();
    let mut failures = Vec::new();
    for line in include_str!("../../../tests/place-names.jsonl").lines().filter(|l| !l.trim().is_empty()) {
        let c: Value = serde_json::from_str(line).unwrap();
        let q = c["query"].as_str().unwrap();
        let cands: Vec<&str> = c["candidates"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
        let categorized: Vec<bool> = c.get("categorized").map_or(vec![false; cands.len()], |v| v.as_array().unwrap().iter().map(|x| x.as_bool().unwrap()).collect());
        let rows: Vec<Row> = cands.iter().zip(&categorized).map(|(&text, &categorized)| Row { text, categorized }).collect();
        let usable: Vec<usize> = (0..cands.len()).filter(|&i| !m.is_control(q, cands[i])).collect();
        let read = c["mode"] == "read";
        let named = !q.split(',').next().unwrap().chars().any(|ch| ch.is_ascii_digit());
        let e = m.pick_explained(q, &rows, &PickOptions { country: Some("US") });
        let answer: (&str, Option<usize>) = match e.outcome {
            Outcome::Match => ("match", e.index),
            _ if usable.is_empty() => ("none", None),
            _ if read && e.prefer_on_read.is_some() => ("match", e.prefer_on_read),
            // a read of a named place takes the site's first usable row in the caller's area, unless
            // that row shows the typed text after words of its own
            _ if read && named => match usable.first().filter(|&&i| !m.shows_typed_text(q, cands[i], 1) && m.in_area(q, cands[i], Some("US"))) {
                Some(&i) => ("match", Some(i)),
                None => ("ambiguous", None),
            },
            // a read of a street address accepts the caller's place under another postcode
            _ if read => match usable.iter().find(|&&i| m.same_but_postcode(q, cands[i], Some("US"))) {
                Some(&i) => ("match", Some(i)),
                None => ("none", None),
            },
            _ => ("ambiguous", None),
        };
        let want = c["picker"].as_str().unwrap();
        let accept: Vec<usize> = c["picker_accept"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as usize).collect();
        let ok = answer.0 == want && (want != "match" || answer.1.map_or(false, |i| accept.contains(&i)));
        if !ok {
            failures.push(format!("{} {:?}", c["id"].as_str().unwrap(), answer));
        }
    }
    assert!(failures.is_empty(), "picker answers differ: {:?}", failures);
}
