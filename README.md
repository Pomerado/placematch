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

## Explained picks for address pickers

`Matcher::pick_explained(query, rows, options)` answers the same question for a picker, and says
why:

```rust
use placematch::{Matcher, Outcome, PickOptions, Row};

let m = Matcher::new();
let rows = [Row::from("Example Hotel, 10 Example St, Springfield, IL 62701"), Row::from("10 Example St, Springfield, IL 62701")];
let e = m.pick_explained("10 Example St, Apt 4, Springfield, IL", &rows, &PickOptions { country: Some("US") });
assert_eq!(e.outcome, Outcome::Match);
assert_eq!(e.index, Some(1)); // PlaceMatch's own choice
assert_eq!(e.same, vec![0, 1]); // every row that is the caller's place, in site order
assert_eq!(e.notes, vec!["unit_not_listed"]); // type the unit into the site's own unit field
```

- `outcome`: `Match`, `None` or `Ambiguous`.
- `same`: for a match, every row that is the caller's place, in the site's order, so a picker can
  take the site's own first among them; for `ambiguous`, the rows to choose between.
- `rows`: each row's verdict and reasons (`house_number`, `postcode`, `place_name`, `area`,
  `control` and so on).
- `reasons`, for `ambiguous`: `several_places` (two places fit), `place_name_variant` (a row holds
  the caller's place name with other words, so it may be the place), and, when no row is the place,
  the near rows: `unit_not_given` (a unit the caller did not give), `postcode_differs` (the caller's
  number, street and town under another postcode), `number_not_listed` (the caller's street with no
  house number) and `number_range` (a house-number range holding the caller's number, with no
  postcode on both sides to confirm it).
- `notes`, for a match: `unit_not_listed` when the row leaves out the unit the caller gave.
- `cross_script`: the match was read across scripts ("Ulitsa Primer" for "Улица Пример").
- `prefer_on_read`: see below.

`pick` gives the same answer, except that it says `none` where only near rows other than
`unit_not_given` were found: it answers whether a row is the place; `pick_explained` also says what
a picker should ask about. A listing's house-number range ("10-14 Example St") holding the caller's
number on the caller's street is the caller's building when both give the same postcode.

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

When a row has exactly the caller's name and another holds the name in a longer form (a pin dropped
under the name next to the venue under its official name), the answer is `ambiguous`. A row may say
that the site shows it with a category of its own (`Row { text, categorized: true }`, a venue type
such as "Convention center"). When exactly one of those rows is categorized, `prefer_on_read` names
it: a picker reading the site may take it, and a write still asks.

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
tools/build-js.sh && node --test js/placematch.test.mjs && tools/check-package.sh
```

## JavaScript

The repository is also a Node ESM package, `@pomerado/placematch`, made of the WebAssembly module and
a small loader that reads no network and no files but the module beside it:

```js
import { open } from "@pomerado/placematch";

const m = await open();
const e = m.pickExplained("example center", [
  "Example Center, Othertown, NY, USA",
  { text: "Jane Q. Example Convention Center, 429 Sample Ave, Exampleton, NY 10001, USA", categorized: true },
], { country: "US" });
// e.outcome === "ambiguous", e.reasons == ["place_name_variant"], e.prefer_on_read === 1
```

`pickExplained` returns the fields above in snake case (`prefer_on_read`, `cross_script`); `pick`
and `compare` are there too. `@pomerado/placematch/loader` exports `load(bytes)` for other
runtimes, and `@pomerado/placematch/placematch.wasm` is the module. `js/placematch.d.ts` has the
types.

The module is built, not tracked on `main`: `tools/build-js.sh` builds it into `js/placematch.wasm`,
and a release commits it, so a dependency pinned to a release tag
(`"@pomerado/placematch": "github:Pomerado/placematch#v0.2.0"`) gets the module with no build step.
`tools/check-package.sh` packs the package and loads it by name.

## Data

The embedded tables come from libpostal (MIT), Google libaddressinput (Apache-2.0) and
OpenCage address-formatting (MIT); see [NOTICE](NOTICE). `tools/vendor-data.sh` refreshes them.

## Contributing

Issues are welcome. Code contributions come only from the maintainers; see
[CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT. See [LICENSE](LICENSE).
