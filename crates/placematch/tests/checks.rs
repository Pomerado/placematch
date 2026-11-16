//! The picker checks (`is_control`, `same_but_postcode`), and no panic on any input. Addresses are
//! made up, apart from public city and province names.
use placematch::{Matcher, PickOptions, Row};

#[test]
fn only_clear_ui_controls_are_controls() {
    let m = Matcher::new();
    for (q, row) in [
        ("Example Park, Springfield", "See more results for Example Park, Springfield"),
        ("Example Park, Springfield", "Search for Example Park, Springfield (12)"),
        ("Example Park, Springfield", "See all 12 results for Example Park, Springfield"),
        ("Example Park, Springfield", "Search \"Example Park, Springfield\""),
        ("Example Center, Springfield", "Use \"Example Center, Springfield\""),
        ("12 Main St, Springfield", "See more results for 12 Main St, Springfield"),
        ("Example Park", "Search for Example Park"),
        ("Example Park", "Example Park - See all results"),
        ("Example Park", "Example Park (see more)"),
    ] {
        assert!(m.is_control(q, row), "{} / {}", q, row);
    }
    for (q, row) in [
        ("Example Park, Springfield", "Jane Doe Example Park, Springfield"),
        ("Example Park, Springfield", "Old Town Example Park, Springfield, IL"),
        ("Main St, Springfield", "Old Town Main St, Springfield"),
        ("Central Station, Exampleton", "Exampleton Central Station, Exampleton"),
        ("Union Station, Springfield", "Springfield Union Station, Springfield"),
        ("Example Center, Springfield, IL", "Jane Q. Example Center, Springfield, IL"),
        ("Example Park", "Example Park, Springfield, IL"),
        ("Example Center, Springfield", "Use my current location"),
    ] {
        assert!(!m.is_control(q, row), "{} / {}", q, row);
    }
}

#[test]
fn another_postcode_is_the_same_building_only_in_the_same_town_state_and_postal_area() {
    let m = Matcher::new();
    let us = Some("US");
    assert!(m.same_but_postcode("10 Main St, Springfield, IL 62701", "10 Main St, Springfield, IL 62702", us));
    for row in ["10 Main St, Springfield, MO 65801", "10 Main St, Springfield, Missouri 65801", "10 Main St, Shelbyville, IL 62702", "10 Main St, Springfield, IL 65801"] {
        assert!(!m.same_but_postcode("10 Main St, Springfield, IL 62701", row, us), "{}", row);
    }
    assert!(!m.same_but_postcode("10 Main St, Springfield, Illinois 62701", "10 Main St, Springfield, MO 65801", us));
    assert!(!m.same_but_postcode("10 Main St, Springfield 62701", "10 Main St, Springfield 65801", us));
    // a city that repeats street names across its districts needs the same district on both
    assert!(!m.same_but_postcode("Example Str. 5, 10115 Berlin", "Example Str. 5, 10117 Berlin", Some("DE")));
    assert!(m.same_but_postcode("Example Str. 5, 10115 Berlin, Mitte", "Example Str. 5, 10117 Berlin, Mitte", Some("DE")));
    assert!(!m.same_but_postcode("Example Str. 5, 10115 Berlin, Mitte", "Example Str. 5, 10117 Berlin, Pankow", Some("DE")));
}

/// Inputs that once crashed: a Han name followed by a lone long-vowel or small-tsu mark.
#[test]
fn a_lone_kana_mark_after_han_does_not_crash() {
    let m = Matcher::new();
    for (q, row, cc) in [
        ("12 Higashi, Exampleshi", "東ー 12", Some("JP")),
        ("Higashi", "東ー", Some("JP")),
        ("1 Example, Higashi", "1 Example, 東ッ", Some("JP")),
        ("9-9-1 Reimachi, Rei-ku, Tokyo", "東京都例区例町ー9ー9ー1", Some("JP")),
        ("5 Sakura, Exampleshi", "桜ー 5, 例市", Some("JP")),
        ("5 Example St, Taichung", "5 Example St, 台中ー", None),
        ("Sakura 5", "桜ー5", None),
        ("Sakura", "桜っ", None),
    ] {
        let _ = m.pick(q, &[row], cc);
        let _ = m.compare(q, row, cc);
        let _ = m.pick_explained(q, &[Row::from(row)], &PickOptions { country: cc });
    }
}

/// No input panics: random strings mixing scripts, marks, digits and separators, through every
/// public call.
#[test]
fn no_input_panics() {
    let m = Matcher::new();
    let pool: Vec<char> = "aeiouxzhqcsjnkgt AEZ 0123456789-/,.#'\"()~ーッっャゅ東京都台中南市区路号丁目番地桜旭町서울로길번ถนนซอยулицаΟδός שדרות شارع".chars().collect();
    let countries = [None, Some("US"), Some("JP"), Some("TW"), Some("CN"), Some("KR"), Some("TH"), Some("RU"), Some("DE"), Some("AU")];
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let text = |next: &mut dyn FnMut() -> u64| -> String {
        let n = (next() % 24) as usize;
        (0..n).map(|_| pool[(next() % pool.len() as u64) as usize]).collect()
    };
    for _ in 0..4000 {
        let q = text(&mut next);
        let rows: Vec<String> = (0..(next() % 4) as usize).map(|_| text(&mut next)).collect();
        let cc = countries[(next() % countries.len() as u64) as usize];
        let rs: Vec<Row> = rows.iter().map(Row::from).collect();
        let _ = m.pick(&q, &rows, cc);
        let _ = m.pick_explained(&q, &rs, &PickOptions { country: cc }).to_json();
        let _ = m.normalize(&q, cc);
        for r in &rows {
            let _ = m.compare(&q, r, cc);
            let _ = m.is_control(&q, r);
            let _ = m.shows_typed_text(&q, r, 1);
            let _ = m.in_area(&q, r, cc);
            let _ = m.same_but_postcode(&q, r, cc);
        }
    }
    for edge in ["", " ", ",", ",,,", "-", "0-0", "ー", "ッ", "東ー", "ーー", "市", "1-2-3-4-5-6-7-8", "\u{0}", "\u{FEFF}", "Ꭺ", "🏠 1 Main St"] {
        let _ = m.pick(edge, &[edge, "1 Main St"], None);
        let _ = m.compare(edge, edge, Some("JP"));
        let _ = m.pick_explained(edge, &[Row::from(edge)], &PickOptions { country: Some("TW") });
    }
}

/// The crate carries the Unicode license and notice for the data it embeds, the same text as the
/// repository's.
#[test]
fn the_crate_carries_its_data_licenses() {
    assert_eq!(include_str!("../LICENSE-UNICODE"), include_str!("../../../LICENSE-UNICODE"));
    assert_eq!(include_str!("../NOTICE"), include_str!("../../../NOTICE"));
}
