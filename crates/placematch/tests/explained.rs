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

fn explain_in(m: &Matcher, country: Option<&str>, query: &str, rows: &[&str]) -> Explained {
    let rows: Vec<Row> = rows.iter().map(|&t| Row::from(t)).collect();
    m.pick_explained(query, &rows, &PickOptions { country })
}

#[test]
fn a_house_number_range_holding_the_number_matches_under_the_same_postcode() {
    let m = Matcher::new();
    let q = "12 Example St, Springfield, IL 62701";
    let e = explain(&m, q, &["10-14 Example St, Springfield, IL 62701"]);
    assert_eq!((e.outcome, e.index), (Outcome::Match, Some(0)));
    assert!(e.rows[0].reasons.contains(&"house_number_range"));
    // another postcode asks, as for a single number
    assert_eq!(explain(&m, q, &["10-14 Example St, Springfield, IL 62702"]).reasons, vec!["postcode_differs"]);
    // no postcode on both sides to confirm the range: ask
    let e = explain(&m, "12 Example St, Springfield", &["10-14 Example St, Springfield"]);
    assert_eq!((e.outcome, e.same.clone(), e.reasons.clone()), (Outcome::Ambiguous, vec![0], vec!["number_range"]));
    assert_eq!(explain(&m, q, &["10-14 Example St, Springfield"]).reasons, vec!["number_range"]);
    // a range listing and the number itself are one place; the number's own row is taken
    let e = explain(&m, q, &["10-14 Example St, Springfield, IL 62701", "12 Example St, Springfield, IL 62701"]);
    assert_eq!((e.outcome, e.index, e.same.clone()), (Outcome::Match, Some(1), vec![0, 1]));
}

/// Two different listed ranges that both hold the caller's number contradict each other: ask.
/// Identical ranges are one place.
#[test]
fn two_different_ranges_holding_the_number_ask() {
    let m = Matcher::new();
    let q = "12 Example St, Springfield, IL 62701";
    let e = explain(&m, q, &["10-14 Example St, Springfield, IL 62701", "12-16 Example St, Springfield, IL 62701"]);
    assert_eq!((e.outcome, e.same.clone(), e.reasons.clone()), (Outcome::Ambiguous, vec![0, 1], vec!["several_places"]));
    let e = explain(&m, q, &["Example Hotel, 10-14 Example St, Springfield, IL 62701", "10-14 Example St, Springfield, IL 62701"]);
    assert_eq!((e.outcome, e.index, e.same.clone()), (Outcome::Match, Some(1), vec![0, 1]));
}

/// A hyphenated pair is read as a range only where it means one, so a range never makes another
/// building the caller's: outside that, the pair is an ordinary house number.
#[test]
fn a_hyphenated_number_is_a_range_only_where_it_means_one() {
    let m = Matcher::new();
    let none = |country: Option<&str>, q: &str, row: &str| {
        let e = explain_in(&m, country, q, &[row]);
        assert_eq!((e.outcome, e.index), (Outcome::None, None), "{:?} {} / {}", country, q, row);
    };
    // Australia and New Zealand write a unit and its number "3-10" (unit 3 at number 10)
    none(Some("AU"), "5 Example St, Exampleton NSW 2000", "3-10 Example St, Exampleton NSW 2000");
    none(Some("AU"), "6 Example St, Exampleton NSW 2000", "4-8 Example St, Exampleton NSW 2000");
    none(Some("NZ"), "6 Example Road, Exampleton 1010", "4-8 Example Road, Exampleton 1010");
    // Hawaii writes a zone and a house number ("91-1234"); Queens a block and a lot ("12-34")
    none(Some("US"), "500 Example St, Kapolei, HI 96707", "498-502 Example St, Kapolei, HI 96707");
    none(Some("US"), "500 Example St, Kapolei, HI 96707", "91-1234 Example St, Kapolei, HI 96707");
    none(Some("US"), "20 Example St, Queens, NY 11105", "18-22 Example St, Queens, NY 11105");
    none(Some("US"), "20 Example St, Queens, NY 11105", "12-34 Example St, Queens, NY 11105");
    // no country to say what the hyphen means, given or shown by the texts
    none(None, "12 Example St, Springfield", "10-14 Example St, Springfield");
    none(None, "5 Asahimachi, Kurume", "3-10 Asahimachi, Kurume");
    // both sides of the street, too wide to be one building, not holding it, another street
    none(Some("US"), "12 Example St, Springfield, IL 62701", "10-15 Example St, Springfield, IL 62701");
    none(Some("US"), "11 Example St, Springfield, IL 62701", "10-14 Example St, Springfield, IL 62701");
    none(Some("US"), "150 Example St, Springfield, IL 62701", "100-198 Example St, Springfield, IL 62701");
    none(Some("US"), "20 Example St, Springfield, IL 62701", "10-30 Example St, Springfield, IL 62701");
    none(Some("US"), "12 Example St, Springfield, IL 62701", "16-20 Example St, Springfield, IL 62701");
    none(Some("US"), "12 Example St, Springfield, IL 62701", "10-14 Sample St, Springfield, IL 62701");
    // and none of these is even asked about as a range
    let e = explain_in(&m, Some("AU"), "5 Example St, Exampleton", &["3-10 Example St, Exampleton"]);
    assert_eq!(e.outcome, Outcome::None);
}

/// `pick` and `compare` keep their answers: they never read a range.
#[test]
fn pick_and_compare_never_read_a_range() {
    let m = Matcher::new();
    let (q, row) = ("12 Example St, Springfield, IL 62701", "10-14 Example St, Springfield, IL 62701");
    assert_eq!(m.pick(q, &[row], Some("US")), Pick::None);
    assert_eq!(m.compare(q, row, Some("US")), (Verdict::Different, vec!["house_number"]));
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

const PIN: &str = "Example Center, Exampleton, NY, USA";
const PIN_ELSEWHERE: &str = "Example Center, Othertown, NY, USA";
const VENUE: &str = "Jane Q. Example Convention Center, 429 Sample Ave, Exampleton, NY 10001, USA";

#[test]
fn a_same_name_pin_beside_a_categorized_venue_asks_and_a_read_may_take_the_venue() {
    let m = Matcher::new();
    let o = PickOptions { country: Some("US") };
    let q = "example center, exampleton, ny";
    let e = m.pick_explained(q, &[Row::from(PIN), Row { text: VENUE, categorized: true }], &o);
    assert_eq!((e.outcome, e.same.clone(), e.reasons.clone()), (Outcome::Ambiguous, vec![0, 1], vec!["place_name_variant"]));
    assert_eq!(e.prefer_on_read, Some(1));
    // the categorized one is preferred whichever it is
    let e = m.pick_explained(q, &[Row { text: PIN, categorized: true }, Row::from(VENUE)], &o);
    assert_eq!(e.prefer_on_read, Some(0));
    // only when the caller gave a town or postcode: with no area, a read asks too
    let e = m.pick_explained("example center", &[Row::from(PIN), Row { text: VENUE, categorized: true }], &o);
    assert_eq!((e.outcome, e.prefer_on_read), (Outcome::Ambiguous, None));
    // both or neither categorized: nothing to prefer
    for (a, b) in [(false, false), (true, true)] {
        let e = m.pick_explained(q, &[Row { text: PIN, categorized: a }, Row { text: VENUE, categorized: b }], &o);
        assert_eq!((e.outcome, e.prefer_on_read), (Outcome::Ambiguous, None));
    }
    // two towns, or a row with no town of its own: the rows may be two places, so nothing to prefer
    let e = m.pick_explained("example center", &[Row::from(PIN_ELSEWHERE), Row { text: VENUE, categorized: true }], &o);
    assert_eq!((e.outcome, e.prefer_on_read), (Outcome::Ambiguous, None));
    let e = m.pick_explained("example center, exampleton, ny", &[Row::from("Example Center, NY, USA"), Row { text: VENUE, categorized: true }], &o);
    assert_eq!((e.outcome, e.prefer_on_read), (Outcome::Ambiguous, None));
    // a category never turns a match into anything else
    let e = m.pick_explained("Example Park, Exampleton, CA", &[Row::from("Example Park, Exampleton, CA"), Row { text: "Example Park Ferry Terminal, Exampleton, CA", categorized: true }], &o);
    assert_eq!((e.outcome, e.index, e.prefer_on_read), (Outcome::Match, Some(0), None));
}

/// A read may take a longer-name row only when its name ends with the caller's name ("Jane Q.
/// Example Convention Center" for "Example Center"): one that goes on after it ("The Example Park
/// Hotel", "Jane Q. Example Park Suites") may be another place named after the caller's, so it is
/// asked about and never preferred.
#[test]
fn a_read_prefers_a_longer_name_only_when_it_ends_with_the_callers_name() {
    let m = Matcher::new();
    let o = PickOptions { country: Some("US") };
    let hotel = Row { text: "The Example Park Hotel, 5 Sample Rd, Springfield, IL", categorized: true };
    let e = m.pick_explained("example park, springfield, il", &[Row::from("Example Park, Springfield, IL"), hotel], &o);
    assert_eq!((e.outcome, e.same.clone(), e.prefer_on_read), (Outcome::Ambiguous, vec![0, 1], None));
    let hotel = Row { text: "The Example Park Hotel, 5 Sample Rd, Shelbyville, IL", categorized: true };
    let e = m.pick_explained("example park", &[Row::from("Example Park, Springfield, IL"), hotel], &o);
    assert_eq!((e.outcome, e.prefer_on_read), (Outcome::Ambiguous, None));
    let after = [
        "Jane Q. Example Park Suites", "Jane Q. Example Park Towers", "The Jane Q. Example Park Residences", "Jane Q. Example Park Apartment Homes",
        "Jane Q. Example Park Zoo", "Jane Q. Example Park Pavilion", "Jane Q. Example Park Theater", "Jane Q. Example Park Club",
        "Jane Q. Example Park Center", "Jane Q. Example Park Hotels", "Jane Q. Example Park Stations", "The Example Park Annex West",
    ];
    for name in after {
        let venue = format!("{}, 5 Sample Rd, Springfield, IL 62701", name);
        for q in ["example park, springfield, il", "example park"] {
            let e = m.pick_explained(q, &[Row::from("Example Park, Springfield, IL"), Row { text: &venue, categorized: true }], &o);
            assert_eq!((e.outcome, e.same.clone(), e.prefer_on_read), (Outcome::Ambiguous, vec![0, 1], None), "{} / {}", q, name);
        }
    }
    // a direction or age word before the name may name another place ("North Example Park")
    for name in ["North Example Park", "Old Example Park", "Upper Example Park", "NE Example Park", "Little Example Park", "New Example Park"] {
        let venue = format!("{}, 5 Sample Rd, Springfield, IL 62701", name);
        let e = m.pick_explained("example park, springfield, il", &[Row::from("Example Park, Springfield, IL"), Row { text: &venue, categorized: true }], &o);
        assert_eq!((e.outcome, e.prefer_on_read), (Outcome::Ambiguous, None), "{}", name);
    }
    let e = m.pick_explained("parc exemple, exempleville", &[Row::from("Parc Exemple, Exempleville, QC"), Row { text: "Vieux Parc Exemple, 5 Rue Modèle, Exempleville, QC", categorized: true }], &PickOptions { country: Some("CA") });
    assert_eq!((e.outcome, e.prefer_on_read), (Outcome::Ambiguous, None));
    // given names and initials before the name are fine, and so is a type word the caller typed
    let e = m.pick_explained("example park, springfield, il", &[Row::from("Example Park, Springfield, IL"), Row { text: "Jane Q. Example Park, 5 Sample Rd, Springfield, IL 62701", categorized: true }], &o);
    assert_eq!((e.outcome, e.prefer_on_read), (Outcome::Ambiguous, Some(1)));
    let e = m.pick_explained("example memorial hospital, exampleton", &[Row::from("Example Memorial Hospital, Exampleton, NY"), Row { text: "Jane Q. Example Memorial Hospital, 5 Sample Rd, Exampleton, NY 10001", categorized: true }], &o);
    assert_eq!((e.outcome, e.prefer_on_read), (Outcome::Ambiguous, Some(1)));
}

/// A read takes the venue the site categorized over a same-name pin only when the caller
/// gave a town or postcode and the venue holds it; with no area from the caller, a read asks.
#[test]
fn a_read_takes_the_venue_only_in_the_callers_area() {
    let m = Matcher::new();
    let o = PickOptions { country: Some("US") };
    let venue = Row { text: "Jane Q. Example Convention Center, 5 Sample Rd, Springfield, MO", categorized: true };
    for pin in ["Example Center, Springfield", "Example Center, Springfield, MO", "Example Center, Springfield, IL"] {
        let e = m.pick_explained("example center", &[Row::from(pin), venue], &o);
        assert_eq!((e.outcome, e.prefer_on_read), (Outcome::Ambiguous, None), "{}", pin);
    }
    let e = m.pick_explained("example center, springfield, mo", &[Row::from("Example Center, Springfield, MO"), venue], &o);
    assert_eq!((e.outcome, e.prefer_on_read), (Outcome::Ambiguous, Some(1)));
    // the exact name only in another town (a borough of its own), the venue in the caller's
    let rows = [Row::from("Example Convention Center, Othertown, NY"), Row { text: "Jane Q. Example Convention Center, 429 Sample Ave, Exampleton, NY 10001", categorized: true }];
    let e = m.pick_explained("example convention center, exampleton, ny", &rows, &o);
    assert_eq!((e.outcome, e.prefer_on_read), (Outcome::Ambiguous, Some(1)));
    let e = m.pick_explained("example convention center", &rows, &o);
    assert_eq!((e.outcome, e.prefer_on_read), (Outcome::Ambiguous, None));
    // the exact name on a street of its own a mile away, the venue in the caller's town too
    let pin = Row::from("Example Tower, 100 Other St, Exampleton, IL 60601");
    let tower = Row { text: "Jane Q. Example Tower, 5 Sample Rd, Exampleton, IL 60602", categorized: true };
    assert_eq!(m.pick_explained("example tower, exampleton, il", &[pin, tower], &o).prefer_on_read, Some(1));
    assert_eq!(m.pick_explained("example tower, exampleton, il", &[tower, pin], &o).prefer_on_read, Some(0));
    let e = m.pick_explained("example tower", &[pin, tower], &o);
    assert_eq!((e.outcome, e.prefer_on_read), (Outcome::Ambiguous, None));
    // known gap, accepted: a longer name with a word inside ("Example Dog Park") in the caller's
    // town is taken on a read as if it were the caller's place under its full name
    let e = m.pick_explained("example park, exampleton, il", &[Row::from("Example Park, Exampleton, IL"), Row { text: "Example Dog Park, 5 Sample Rd, Exampleton, IL 60602", categorized: true }], &o);
    assert_eq!((e.outcome, e.prefer_on_read), (Outcome::Ambiguous, Some(1)));
}

/// A same-name row in another town beside a venue whose longer name ends in a place-type word is
/// still a question: `pick`, `compare` and `pick_explained` all keep asking.
#[test]
fn a_type_word_venue_beside_a_same_name_row_elsewhere_still_asks() {
    let m = Matcher::new();
    let cases: [(&str, &str, [&str; 2]); 7] = [
        ("US", "example memorial", ["Example Memorial, Othertown, NY", "Jane Q. Example Memorial Hospital, 5 Sample Rd, Exampleton, NY 10001"]),
        ("US", "example square", ["Example Square, Othertown, NY", "Jane Q. Example Square Garden, Exampleton, NY"]),
        ("US", "example international", ["Example International, Othertown, CA", "Exampleton Example International Airport, Exampleton, CA"]),
        ("US", "example central", ["Example Central, Othertown, NY", "Exampleton Example Central Terminal, Exampleton, NY"]),
        ("US", "example field", ["Example Field, Othertown, NY", "Jane Q. Example Field Stadium, Exampleton, NY"]),
        ("US", "example cross", ["Example Cross, Othertown", "Exampleton Example Cross Station, Exampleton"]),
        ("GB", "example cross", ["Example Cross, Othertown", "Exampleton Example Cross Station, Exampleton"]),
    ];
    for (cc, q, rows) in cases {
        let e = m.pick_explained(q, &[Row::from(rows[0]), Row { text: rows[1], categorized: true }], &PickOptions { country: Some(cc) });
        assert_eq!((e.outcome, e.same.clone(), e.prefer_on_read), (Outcome::Ambiguous, vec![0, 1], None), "{} {}", cc, q);
        assert_eq!(m.pick(q, &rows, Some(cc)), Pick::Ambiguous(vec![0, 1]), "{} {}", cc, q);
        assert_eq!(m.compare(q, rows[1], Some(cc)).0, Verdict::Unknown, "{} {}", cc, q);
    }
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

/// A picker that takes the site's first row of the place still learns the unit is not listed there.
#[test]
fn each_row_of_the_place_carries_its_own_notes() {
    let m = Matcher::new();
    let e = explain(&m, "10 Example St, Apt 4, Springfield, IL", &["10 Example St, Springfield, IL", "10 Example St, Apt 4, Springfield, IL"]);
    assert_eq!((e.outcome, e.index, e.same.clone(), e.notes.clone()), (Outcome::Match, Some(1), vec![0, 1], vec![]));
    assert_eq!(e.rows[0].notes, vec!["unit_not_listed"]);
    assert!(e.rows[1].notes.is_empty());
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
            "rows": [{"verdict": "same", "reasons": [], "notes": ["unit_not_listed"]}, {"verdict": "different", "reasons": ["house_number"], "notes": []}],
            "unit": "Apt 4", "inferred_country": null
        })
    );
}

/// `pick` answers as `pick_explained` does wherever it should: the same row on a match, the same
/// rows on an ambiguous answer, and `none` (or the rows with an added unit) where `pick_explained`
/// asks only about near rows. Ranges are read by `pick_explained` alone.
/// In a city whose street names repeat, another postcode in the caller's district ("W1D") may be a
/// mistyped postcode, so it asks; one in another district is another street of the name.
#[test]
fn a_postcode_in_the_callers_district_asks() {
    let m = Matcher::new();
    let ask = |row: &str| m.pick_explained("1 Example St, London W1D 1AN", &[Row::from(row)], &PickOptions { country: Some("GB") });
    for row in ["1 Example St, London W1D 2HP", "1 Example St, London W1D 1BB"] {
        let e = ask(row);
        assert_eq!((e.outcome, e.reasons.clone()), (Outcome::Ambiguous, vec!["postcode_differs"]), "{}", row);
    }
    for row in ["1 Example St, London W1F 1AA", "1 Example St, London SW1A 1AA"] {
        assert_eq!(ask(row).outcome, Outcome::None, "{}", row);
    }
}

#[test]
fn pick_agrees_with_pick_explained() {
    let m = Matcher::new();
    let near = ["unit_not_given", "postcode_differs", "number_not_listed", "number_range"];
    for text in [include_str!("../../../tests/cases.jsonl"), include_str!("../../../tests/place-names.jsonl")] {
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let c: serde_json::Value = serde_json::from_str(line).unwrap();
            let q = c["query"].as_str().unwrap();
            let cc = c["country"].as_str();
            let cands: Vec<&str> = c["candidates"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
            let rows: Vec<Row> = cands.iter().map(|&t| Row::from(t)).collect();
            let e = m.pick_explained(q, &rows, &PickOptions { country: cc });
            if e.rows.iter().any(|r| r.reasons.iter().any(|x| matches!(*x, "house_number_range" | "number_range"))) {
                continue;
            }
            let want = match e.outcome {
                Outcome::Match => Pick::Match(e.index.unwrap()),
                Outcome::None => Pick::None,
                Outcome::Ambiguous if e.reasons.iter().all(|r| near.contains(r)) => {
                    let units: Vec<usize> = e.same.iter().copied().filter(|&i| e.rows[i].reasons.contains(&"unit_not_given")).collect();
                    if units.is_empty() {
                        Pick::None
                    } else {
                        Pick::Ambiguous(units)
                    }
                }
                Outcome::Ambiguous => Pick::Ambiguous(e.same.clone()),
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

/// The caller's unit comes back as the caller wrote it, whatever the outcome, so a picker can type
/// it into the site's own unit field.
#[test]
fn the_callers_unit_comes_back_as_written() {
    let m = Matcher::new();
    let unit = |q: &str| explain(&m, q, &["1 Example St, Exampleton, IL 60000"]).unit;
    assert_eq!(unit("350 Example Ave Apt 3, Exampleton, IL 60000").as_deref(), Some("Apt 3"));
    assert_eq!(unit("1 Example St #4B, Exampleton, IL").as_deref(), Some("#4B"));
    assert_eq!(unit("Suite 200, 1 Example St, Exampleton, IL").as_deref(), Some("Suite 200"));
    assert_eq!(unit("1 Example St, Fl 2, Exampleton, IL").as_deref(), Some("Fl 2"));
    assert_eq!(unit("1 Example St, Exampleton, IL 60000"), None);
    // a state code before a ZIP code is no unit
    assert_eq!(unit("1 Example St, Exampleton, FL 32500"), None);
}

/// With no country given, the texts are read by the one country they show, and never against a
/// country the caller gave.
#[test]
fn a_country_the_texts_show_is_read_when_none_is_given() {
    let m = Matcher::new();
    let inferred = |q: &str, rows: &[&str]| explain_in(&m, None, q, rows).inferred_country;
    // a state with its ZIP code, a province with its postcode, a country's name
    assert_eq!(inferred("1 Example St, Exampleton, IL 60000", &["1 Example St, Exampleton, IL 60000"]).as_deref(), Some("US"));
    assert_eq!(inferred("1 Example St, Exampleton, ON K1A 0B1", &["1 Example St, Exampleton, ON K1A 0B1"]).as_deref(), Some("CA"));
    assert_eq!(inferred("Rue Exemple 5, 1000 Bruxelles, Belgium", &["Rue Exemple 5, 1000 Bruxelles"]).as_deref(), Some("BE"));
    // the rows show it and the caller's state or ZIP code fits it, or the caller gives no place
    assert_eq!(inferred("Example Park, Exampleton, CA", &["Example Park, Exampleton, CA, USA"]).as_deref(), Some("US"));
    assert_eq!(inferred("1 Example St, Exampleton, FL", &["1 Example St, Exampleton 32500"]).as_deref(), Some("US"));
    assert_eq!(inferred("12 Example St", &["12 Example St, Exampleton, IL 60000"]).as_deref(), Some("US"));
    // the caller's own country decides, and a row that shows another is another place
    assert_eq!(inferred("1 Example St, Exampleton, IL 60000", &["1 Example St, Exampleton, ON, Canada"]).as_deref(), Some("US"));
    let e = explain_in(&m, None, "1 Example St, Exampleton, IL 60000", &["1 Example St, Exampleton, ON, Canada"]);
    assert_eq!((e.outcome, e.rows[0].reasons.clone()), (Outcome::None, vec!["country"]));
    assert_eq!(inferred("Example Park, London, United Kingdom", &["Example Park, London, Ontario"]).as_deref(), Some("GB"));
    // rows of two countries and nothing from the caller, or too little to tell: none, and the
    // texts are read with no country
    assert_eq!(inferred("12 Example St, Exampleton", &["12 Example St, Exampleton, IL 60000", "12 Example St, Exampleton, ON K1A 0B1"]), None);
    assert_eq!(inferred("12 Example St, Springfield", &["12 Example St, Springfield"]), None);
    assert_eq!(inferred("Example Park, Exampleton, CA", &["Example Park, Exampleton, CA"]), None);
    // a country the caller gives is never replaced
    let e = explain_in(&m, Some("CA"), "1 Example St, Exampleton, IL 60000", &["1 Example St, Exampleton, IL 60000"]);
    assert_eq!(e.inferred_country, None);
    // read as the United States, the hyphenated listing is a range holding the caller's number
    let e = explain_in(&m, None, "12 Example St, Springfield, IL 62701", &["10-14 Example St, Springfield, IL 62701"]);
    assert_eq!((e.outcome, e.index), (Outcome::Match, Some(0)));
}

/// A ZIP code tells the zone of states it is in: one outside the caller's state is another place,
/// one inside it confirms the state a row leaves out.
#[test]
fn a_zip_code_outside_the_callers_state_is_another_place() {
    let m = Matcher::new();
    let e = explain(&m, "1 Example St, Exampleton, PA", &["1 Example St, Exampleton 60000"]);
    assert_eq!(e.outcome, Outcome::None);
    let e = explain(&m, "1 Example St, Exampleton, CA", &["1 Example St, Exampleton 93600"]);
    assert_eq!((e.outcome, e.index), (Outcome::Match, Some(0)));
    // a row that leaves out only the state, with nothing that contradicts the caller, is taken
    let e = explain(&m, "1 Example St, Exampleton, IL 60000", &["1 Example St, Exampleton"]);
    assert_eq!((e.outcome, e.index), (Outcome::Match, Some(0)));
}
