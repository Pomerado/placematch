//! `pick_explained`: the outcome with the rows that are the caller's place, the reasons and notes a
//! picker needs, and what a read may take. Addresses are made up.
use placematch::{Explained, Matcher, Outcome, Pick, PickOptions, Row, Verdict};

fn explain(m: &Matcher, query: &str, rows: &[&str]) -> Explained {
    let rows: Vec<Row> = rows.iter().map(|&t| Row::from(t)).collect();
    m.pick_explained(query, &rows, &PickOptions { country: Some("US") })
}

#[test]
fn a_match_lists_every_row_of_the_place_in_site_order() {
    let m = Matcher::new();
    let rows = ["Example Hotel, 10 Example St, Springfield, IL 62701", "12 Example St, Springfield, IL 62701", "10 Example Street, Springfield, IL 62701"];
    let e = explain(&m, "10 Example St, Springfield, IL 62701", &rows);
    assert_eq!(e.outcome, Outcome::Match);
    // the plain listing is PlaceMatch's own choice; the business listing at the address is the same place
    assert_eq!(e.index, Some(2));
    assert_eq!(e.same, vec![0, 2]);
    assert!(e.reasons.is_empty() && e.notes.is_empty() && !e.cross_script && e.prefer_on_read.is_none());
    assert_eq!(e.rows[1].verdict, Verdict::Different);
    assert!(e.rows[1].reasons.contains(&"house_number"));
    assert_eq!(e.rows[0].verdict, Verdict::Same);
}

#[test]
fn a_listing_that_leaves_out_the_callers_unit_is_a_match_with_a_note() {
    let m = Matcher::new();
    let e = explain(&m, "10 Example St, Apt 4, Springfield, IL", &["10 Example St, Springfield, IL 62701"]);
    assert_eq!((e.outcome, e.index, e.notes.clone()), (Outcome::Match, Some(0), vec!["unit_not_listed"]));
    // a listing that keeps the unit is taken, with no note
    let e = explain(&m, "10 Example St, Apt 4, Springfield, IL", &["10 Example St, Springfield, IL 62701", "10 Example St, Apt 4, Springfield, IL 62701"]);
    assert_eq!((e.outcome, e.index, e.notes.len()), (Outcome::Match, Some(1), 0));
}

#[test]
fn a_unit_the_caller_did_not_give_asks() {
    let m = Matcher::new();
    let e = explain(&m, "10 Example St, Springfield", &["10 Example St, Apt 4, Springfield", "11 Example St, Springfield"]);
    assert_eq!((e.outcome, e.same.clone(), e.reasons.clone()), (Outcome::Ambiguous, vec![0], vec!["unit_not_given"]));
}

#[test]
fn another_postcode_at_the_same_number_and_street_asks() {
    let m = Matcher::new();
    let q = "10 Example St, Springfield, IL 62701";
    let e = explain(&m, q, &["10 Example St, Springfield, IL 62702", "11 Example St, Springfield, IL 62701"]);
    assert_eq!((e.outcome, e.same.clone(), e.reasons.clone()), (Outcome::Ambiguous, vec![0], vec!["postcode_differs"]));
    // `pick` still answers whether a row is the place: none of them is
    assert_eq!(m.pick(q, &["10 Example St, Springfield, IL 62702"], Some("US")), Pick::None);
    // the right postcode among them is the match
    let e = explain(&m, q, &["10 Example St, Springfield, IL 62702", "10 Example St, Springfield, IL 62701"]);
    assert_eq!((e.outcome, e.index), (Outcome::Match, Some(1)));
}

#[test]
fn a_listing_of_the_street_without_the_callers_number_asks() {
    let m = Matcher::new();
    let e = explain(&m, "10 Example Square, Springfield", &["Example Square, Springfield"]);
    assert_eq!((e.outcome, e.same.clone(), e.reasons.clone()), (Outcome::Ambiguous, vec![0], vec!["number_not_listed"]));
    // a town alone is not the street
    assert_eq!(explain(&m, "10 Example Square, Springfield", &["Springfield, IL"]).outcome, Outcome::None);
}

#[test]
fn a_house_number_range_holding_the_number_matches_under_the_same_postcode() {
    let m = Matcher::new();
    let q = "12 Example St, Springfield, IL 62701";
    let e = explain(&m, q, &["10-14 Example St, Springfield, IL 62701"]);
    assert_eq!((e.outcome, e.index), (Outcome::Match, Some(0)));
    assert_eq!(m.pick(q, &["10-14 Example St, Springfield, IL 62701"], Some("US")), Pick::Match(0));
    assert_eq!(explain(&m, q, &["10–14 Example Street, Springfield, IL 62701"]).outcome, Outcome::Match);
    // another postcode asks, as for a single number
    assert_eq!(explain(&m, q, &["10-14 Example St, Springfield, IL 62702"]).reasons, vec!["postcode_differs"]);
    // no postcode on either side to confirm the range: ask
    let e = explain(&m, "12 Example St, Springfield", &["10-14 Example St, Springfield"]);
    assert_eq!((e.outcome, e.same.clone(), e.reasons.clone()), (Outcome::Ambiguous, vec![0], vec!["number_range"]));
    let e = explain(&m, q, &["10-14 Example St, Springfield"]);
    assert_eq!(e.reasons, vec!["number_range"]);
    // a range that does not hold it, the other side of the street, or another street
    assert_eq!(explain(&m, q, &["16-20 Example St, Springfield, IL 62701"]).outcome, Outcome::None);
    assert_eq!(explain(&m, "11 Example St, Springfield, IL 62701", &["10-14 Example St, Springfield, IL 62701"]).outcome, Outcome::None);
    assert_eq!(explain(&m, q, &["10-14 Sample St, Springfield, IL 62701"]).outcome, Outcome::None);
    // a range listing and the number itself are one place; the number's own row is taken
    let e = explain(&m, q, &["10-14 Example St, Springfield, IL 62701", "12 Example St, Springfield, IL 62701"]);
    assert_eq!((e.outcome, e.index, e.same.clone()), (Outcome::Match, Some(1), vec![0, 1]));
}

#[test]
fn the_same_number_and_street_in_two_towns_asks() {
    let m = Matcher::new();
    let e = explain(&m, "10 Example St", &["10 Example St, Springfield", "10 Example St, Shelbyville"]);
    assert_eq!((e.outcome, e.same.clone(), e.reasons.clone()), (Outcome::Ambiguous, vec![0, 1], vec!["several_places"]));
}

#[test]
fn a_pick_read_across_scripts_says_so() {
    let m = Matcher::new();
    let o = PickOptions { country: Some("RU") };
    let e = m.pick_explained("Улица Пример 10, Москва", &[Row::from("Ulitsa Primer 12, Moskva"), Row::from("Ulitsa Primer 10, Moskva")], &o);
    assert_eq!((e.outcome, e.index, e.cross_script), (Outcome::Match, Some(1), true));
    let e = m.pick_explained("Улица Пример 10, Москва", &[Row::from("Улица Пример 10, Москва")], &o);
    assert_eq!((e.outcome, e.cross_script), (Outcome::Match, false));
    let o = PickOptions { country: Some("JP") };
    let e = m.pick_explained("12 Asahicho, Kurume-shi", &[Row::from("久留米市旭町12")], &o);
    assert_eq!((e.outcome, e.cross_script), (Outcome::Match, true));
}

const PIN: &str = "Example Center, Othertown, NY, USA";
const VENUE: &str = "Jane Q. Example Convention Center, 429 Sample Ave, Exampleton, NY 10001, USA";

#[test]
fn a_same_name_pin_beside_a_categorized_venue_asks_and_a_read_may_take_the_venue() {
    let m = Matcher::new();
    let o = PickOptions { country: Some("US") };
    let e = m.pick_explained("example center", &[Row::from(PIN), Row { text: VENUE, categorized: true }], &o);
    assert_eq!((e.outcome, e.same.clone(), e.reasons.clone()), (Outcome::Ambiguous, vec![0, 1], vec!["place_name_variant"]));
    assert_eq!(e.prefer_on_read, Some(1));
    // the categorized one is preferred whichever it is
    let e = m.pick_explained("example center", &[Row { text: PIN, categorized: true }, Row::from(VENUE)], &o);
    assert_eq!(e.prefer_on_read, Some(0));
    // both or neither categorized: nothing to prefer
    for (a, b) in [(false, false), (true, true)] {
        let e = m.pick_explained("example center", &[Row { text: PIN, categorized: a }, Row { text: VENUE, categorized: b }], &o);
        assert_eq!((e.outcome, e.prefer_on_read), (Outcome::Ambiguous, None));
    }
    // a category never turns a match into anything else
    let e = m.pick_explained("Example Park, Exampleton, CA", &[Row::from("Example Park, Exampleton, CA"), Row { text: "Example Park Ferry Terminal, Exampleton, CA", categorized: true }], &o);
    assert_eq!((e.outcome, e.index, e.prefer_on_read), (Outcome::Match, Some(0), None));
}

#[test]
fn named_rows_say_why_they_are_not_the_place() {
    let m = Matcher::new();
    let rows = ["Example Park, Exampleton, CA", "Example Park Ferry Terminal, Exampleton, CA", "Example Park, Othertown, NV", "See more results for Example Park, Exampleton, CA"];
    let e = explain(&m, "Example Park, Exampleton, CA", &rows);
    assert_eq!((e.outcome, e.index, e.same.clone()), (Outcome::Match, Some(0), vec![0]));
    let why: Vec<(Verdict, Vec<&str>)> = e.rows.iter().map(|r| (r.verdict, r.reasons.clone())).collect();
    assert_eq!(why, vec![(Verdict::Same, vec![]), (Verdict::Different, vec!["place_name"]), (Verdict::Different, vec!["area"]), (Verdict::Different, vec!["control"])]);
}

#[test]
fn the_json_form_carries_every_field() {
    let m = Matcher::new();
    let e = explain(&m, "10 Example St, Apt 4, Springfield, IL", &["10 Example St, Springfield, IL 62701", "11 Example St, Springfield, IL"]);
    let j: serde_json::Value = serde_json::from_str(&e.to_json()).unwrap();
    assert_eq!(
        j,
        serde_json::json!({
            "outcome": "match", "index": 0, "same": [0], "reasons": [], "notes": ["unit_not_listed"],
            "cross_script": false, "prefer_on_read": null,
            "rows": [{"verdict": "same", "reasons": ["unit_not_listed"]}, {"verdict": "different", "reasons": ["house_number"]}]
        })
    );
}

/// `pick` answers as before: the same row, or `none` where only near rows (another postcode, no
/// house number, an unconfirmed range) were found.
#[test]
fn pick_agrees_with_pick_explained() {
    let m = Matcher::new();
    for text in [include_str!("../../../tests/cases.jsonl"), include_str!("../../../tests/place-names.jsonl")] {
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let c: serde_json::Value = serde_json::from_str(line).unwrap();
            let q = c["query"].as_str().unwrap();
            let cc = c["country"].as_str();
            let cands: Vec<&str> = c["candidates"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
            let rows: Vec<Row> = cands.iter().map(|&t| Row::from(t)).collect();
            let e = m.pick_explained(q, &rows, &PickOptions { country: cc });
            let near_only = e.reasons.iter().all(|r| matches!(*r, "postcode_differs" | "number_not_listed" | "number_range"));
            let want = match e.outcome {
                Outcome::Match => Pick::Match(e.index.unwrap()),
                Outcome::None => Pick::None,
                Outcome::Ambiguous if near_only => Pick::None,
                Outcome::Ambiguous => match m.pick(q, &cands, cc) {
                    Pick::Ambiguous(_) => continue,
                    other => panic!("{}: pick {:?}, explained ambiguous", c["id"], other),
                },
            };
            assert_eq!(m.pick(q, &cands, cc), want, "{}", c["id"]);
        }
    }
}

#[test]
fn readme_explained_example() {
    let m = Matcher::new();
    let rows = [Row::from("Example Hotel, 10 Example St, Springfield, IL 62701"), Row::from("10 Example St, Springfield, IL 62701")];
    let e = m.pick_explained("10 Example St, Apt 4, Springfield, IL", &rows, &PickOptions { country: Some("US") });
    assert_eq!(e.outcome, Outcome::Match);
    assert_eq!(e.index, Some(1));
    assert_eq!(e.same, vec![0, 1]);
    assert_eq!(e.notes, vec!["unit_not_listed"]);
}
