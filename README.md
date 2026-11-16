# PlaceMatch

PlaceMatch decides whether an address someone wrote is the same place as an address a
website offers. Give it the caller's text and the site's candidates; it answers with the one
candidate that is the same place, with `none`, or with `ambiguous` when it should ask.

It is a small Rust library with no network access, no model and no runtime data files. It also
builds to a 1.5 MB WebAssembly module.

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
Arabic street names, for example). Japanese and Chinese place names cannot be recovered from a
romanization without a reading dictionary, so a romanized Japanese address compared with a
Japanese-script listing gets `ambiguous` or `none`, never a guess.

## Accuracy

Measured on real addresses held out by region (see [bench/real](bench/real)): whole states,
provinces, source datasets and some entire countries were never used while developing the
matcher. The libpostal baseline uses libpostal v1.1.4's parser and near-duplicate checks with
the same policy.

Held-out split: 76,722 cases from 62 countries, 11 scripts. Cells are accuracy / wrong-pick rate.

| | PlaceMatch | libpostal baseline | both agree |
|---|---|---|---|
| all cases | 91.6% / 0.39% | 75.9% / 1.87% | 74.6% / 0.04% |
| one candidate (`pair`) | 92.7% / 0.17% | 79.3% / 0.44% | 77.6% / 0.01% |
| 2 to 8 candidates (`pick`) | 90.1% / 0.69% | 71.3% / 3.81% | 70.6% / 0.10% |
| government and open-data addresses | 96.2% / 0.09% | 82.6% / 1.28% | 80.9% / 0.02% |
| OpenStreetMap addresses and listings | 89.7% / 0.51% | 73.2% / 2.11% | 72.1% / 0.05% |
| Latin script | 94.7% / 0.40% | 81.1% / 1.34% | 79.3% / 0.04% |
| other scripts | 86.0% / 0.38% | 66.6% / 2.83% | 66.2% / 0.05% |
| no commas | 82.8% / 0.38% | 76.0% / 1.89% | 69.4% / 0.04% |
| transliterated | 53.5% / 0.22% | 35.2% / 0.25% | 34.4% / 0.00% |

"Both agree" answers `match` only when PlaceMatch and libpostal pick the same listing; it trades
coverage for the fewest wrong picks. Most remaining misses are `none` or `ambiguous` answers
where a listing did match: romanized Japanese, Chinese and Thai names (no reading dictionary is
embedded), comma-less addresses where town and region names collide, and label noise in the
source data. The project's goal is 97% with fewer wrong picks than the baseline; the second
part holds today, the first does not yet.

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
