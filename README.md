# PlaceMatch

PlaceMatch decides whether an address someone wrote is the same place as an address a
website offers. Give it the caller's text and the site's candidates; it answers with the one
candidate that is the same place, with `none`, or with `ambiguous` when it should ask.

It is a small Rust library with no network access, no model and no runtime data files. It also
builds to a 2.4 MB WebAssembly module.

```rust
use placematch::{Matcher, Pick};

let m = Matcher::new();
let pick = m.pick(
    "12 Maple Ave Apt 4, Springfield IL",
    &["12 Maple Avenue, Apt 4, Springfield, IL 62704", "120 Maple Avenue, Springfield, IL 62704"],
    Some("US"),
);
assert_eq!(pick, Pick::Match(0));
```

`Matcher::compare(query, candidate, country)` returns `Same`, `Different` or `Unknown` with the
reasons, and `Matcher::normalize` returns the parsed components.

## What counts as the same place

The full policy is in [bench/POLICY.md](bench/POLICY.md). In short:

- House number, street, street type, unit, town, postcode and PO box must not contradict.
  Postcode, region, country, district and business names may be missing on either side.
- A candidate that adds a unit the caller did not give is not picked. When only such
  candidates fit, the answer is `ambiguous`, so the caller can be asked which unit.
- A business listing at the address is the same place as the plain address.
- The town must be right: a shared district or region name does not make two towns one.
- Spellings that mean the same thing agree: abbreviations, diacritics, case and punctuation,
  component order (number before or after the street), joined or split compound names, native
  digits, Japanese block addresses, transliteration, and a one-letter typo in a long name.

Wrong picks are the costly error, so PlaceMatch prefers `ambiguous` or `none` whenever a
candidate could be a different place.

## Scripts and conventions

PlaceMatch reads Latin, Cyrillic, Greek, Arabic, Hebrew, Devanagari, Thai, Hangul, Han and
Kana text. It knows street, unit and level words in every language of the
[libpostal](https://github.com/openvenues/libpostal) dictionaries, postcode formats and region
names for every country from [Google libaddressinput](https://github.com/google/libaddressinput),
and house-number-after-street conventions.

Transliteration works when the romanization keeps the consonants (Cyrillic, Greek, Korean and
Arabic street names, for example). Japanese and Chinese names are read with per-character
readings from the Unicode Han database: a romanization matches when it splits, in order, into one
reading for each character (Hepburn or Kunrei for Japanese, with long vowels written or not and the
sound changes of compound names; Hanyu Pinyin, Wade-Giles or Tongyong for Chinese). Thai names
are compared with their romanizations by consonant sounds. Korean road names ("경원대로1418번길",
"Gyeongwon-daero 1418beon-gil") are read as one word in both scripts. A Japanese place name whose
reading is not built from its characters' listed readings gets `ambiguous` or `none`, never a
guess.

## Places named without a house number

When the caller gives a place's name and no house number ("Example Park, Exampleton, CA"), the
name decides:

- a row whose first part is the same name, word for word (Street for St), is the place, whatever
  street follows the name, when its town, region and postcode fit the caller's;
- a row that holds the caller's words in order with others added before or between them ("The
  Example Park", or a venue under its full name) is `ambiguous`: it may be the place;
- a row that starts with the name and goes on ("Example Park Ferry Terminal") is another place;
- a row that shows the caller's whole text after words of its own ("See more results for ...") is
  a site control, not a place.

A picker around PlaceMatch decides what to do with `none` and `ambiguous` (ask in a write, refuse or
read on in a read). `Matcher::is_control`, `Matcher::in_area` and `Matcher::same_but_postcode`
answer the questions such a picker asks; `tests/place-names.jsonl` holds the cases, with the
picker's answer for each mode beside PlaceMatch's own.

## Accuracy

Measured on real addresses held out by region (see [bench/real](bench/real)): whole states,
provinces, source datasets and some entire countries were never used while developing the
matcher. The libpostal baseline uses libpostal v1.1.4's parser and near-duplicate checks with
the same policy.

Held-out split: 76,722 cases from 62 countries, 11 scripts. Cells are accuracy / wrong-pick rate.

| | PlaceMatch | libpostal baseline | both agree |
|---|---|---|---|
| all cases | 92.5% / 0.37% | 75.9% / 1.87% | 74.9% / 0.04% |
| one candidate (`pair`) | 93.6% / 0.18% | 79.3% / 0.44% | 77.9% / 0.01% |
| 2 to 8 candidates (`pick`) | 91.0% / 0.63% | 71.3% / 3.81% | 70.9% / 0.08% |
| government and open-data addresses | 96.4% / 0.09% | 82.6% / 1.28% | 81.0% / 0.02% |
| OpenStreetMap addresses and listings | 90.9% / 0.49% | 73.2% / 2.11% | 72.4% / 0.04% |
| Latin script | 95.1% / 0.39% | 81.1% / 1.34% | 79.5% / 0.04% |
| other scripts | 87.9% / 0.34% | 66.6% / 2.83% | 66.7% / 0.03% |
| no commas | 84.8% / 0.33% | 76.0% / 1.89% | 70.6% / 0.03% |
| transliterated | 59.0% / 0.25% | 35.2% / 0.25% | 35.1% / 0.00% |

"Both agree" answers `match` only when PlaceMatch and libpostal pick the same listing; it trades
coverage for the fewest wrong picks. Most remaining misses are `none` or `ambiguous` answers
where a listing did match: Chinese addresses that write sections, lanes and alleys in another order,
Japanese and Thai names read irregularly, Hebrew and Cyrillic romanizations, and label noise in the
source data. The project's goal is 97% with fewer wrong picks than the baseline; the second part
holds today, the first does not yet.

A wrong pick is a `match` answer naming a listing that is not the caller's place.

## Building

```sh
cargo test --release
cargo build --release -p placematch-bench    # reads cases on stdin, writes predictions
cargo build --profile wasm --target wasm32-unknown-unknown -p placematch-wasm
```

`js/placematch.mjs` is a small loader for the WebAssembly module.

## Data

The embedded tables come from libpostal (MIT), Google libaddressinput (Apache-2.0) and
OpenCage address-formatting (MIT); see [NOTICE](NOTICE). `tools/vendor-data.sh` refreshes them.

## Contributing

Issues are welcome. Code contributions come only from the maintainers; see
[CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT. See [LICENSE](LICENSE).
