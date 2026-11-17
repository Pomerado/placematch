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
  `control`, `house_number_range` and so on), and, for each row of the place, its notes, so a
  picker that takes the site's own first row of the place still learns that row leaves out the
  caller's unit.
- `reasons`, for `ambiguous`: `several_places` (two places fit), `place_name_variant` (a row holds
  the caller's place name with other words, so it may be the place), and, when no row is the place,
  the near rows: `unit_not_given` (a unit the caller did not give), `postcode_differs` (the caller's
  number, street and town under another postcode in the same area; for a UK postcode, the same
  district, such as "W1D"), `number_not_listed` (the caller's street with no
  house number) and `number_range` (a house-number range holding the caller's number, with no
  postcode on both sides to confirm it).
- `notes`, for a match: `unit_not_listed` when the row at `index` leaves out the unit the caller
  gave (each row's own notes are in `rows`).
- `cross_script`: the match was read across scripts ("Ulitsa Primer" for "Улица Пример").
- `prefer_on_read`: see below.
- `unit`: the caller's unit as the caller wrote it ("Apt 3", "#4B", "Suite 200"), whatever the
  outcome, so a picker can type it into the site's own unit field.
- `inferred_country`: with no country given, the country the texts were read by (below).

### Without a country

A country is given as its ISO 3166-1 code in any case; "UK" is read as "GB", and a code PlaceMatch
does not know is read as no country.

A caller's country is the best guide, but it may be left out. Then PlaceMatch reads the texts by
the one country they show: a country's name ("UK" and "US" included), a state or province with its
postcode ("IL 62701", "ON K1A 0B1"), or a postcode only one country writes ("K1A 0B1", "SW1A 1AA").
The caller's text decides when it shows a country, and a row that shows another is then another
place; otherwise the rows decide when they all show one country and the caller's text fits it (a
state or postcode of it, or no place at all). A US state with a row's ZIP code of that state counts
too. When the texts show nothing, or rows of two countries, they are read with no country, which
asks more often and never guesses: when rows of two countries would each be the place, read by their
own country ("10 Main St, Vancouver" beside Vancouver, WA and Vancouver, BC), the answer asks. A
country the caller gives is never replaced. `Matcher::infer_country` answers on its own.

On the US cases of the benchmark and fixtures, answers with no country equal the answers with `US`
in all but 15 of 1,540 tuning cases, none of 160 held-out cases and 16 of 165 fixtures, and none of
those differences picks a wrong row for a caller who gives no country: most ask where the US answer
matched. Outside the US, pass the country: with none, wrong picks are two to three times those with
it.

`pick` and `compare` keep their answers: they answer whether a row is the place. `pick_explained`
also says what a picker should ask about, so it asks where `pick` says `none` because only near
rows were found, and it alone reads house-number ranges.

A listing's hyphenated house number ("10-14 Example St") is read as a range of one building only
where it means one: the caller's country must be given and be the US, the UK, Ireland, Germany or
France; both sides in Latin script; not Hawaii (ZIPs 967xx and 968xx, zone-and-house numbers) or
Queens-style numbering (ZIPs 110xx to 116xx, block-and-lot numbers); ascending, at most 8 wide, and
with the caller's number on the range's side of the street (both ends and the number all even or all
odd). Then it is the caller's building when both sides give the same postcode, and `number_range`
asks when they don't both give one. Anywhere else the pair is an ordinary house number, so
"3-10 Example St" (unit 3 at number 10 in Australia) is never "5 Example St".

## What counts as the same place

The full policy is in [bench/POLICY.md](bench/POLICY.md). In short:

- House number, street, street type, unit, town, postcode and PO box must not contradict.
  Postcode, region, country, district and business names may be missing on either side.
- A candidate that adds a unit the caller did not give is not picked. When only such
  candidates fit, the answer is `ambiguous`, so the caller can be asked which unit.
- A short part before the address that may be a unit, whatever its word ("№7", "Studio 12",
  "U7", "7F", "7-B", "7"), is read as the row's unit: another unit is not the caller's, and
  otherwise the row is at most asked about, never taken, even beside a row that fits. Any other
  leading part with a number is at most asked about when that number is not the address's and
  the caller gave a unit, or when the part is three words or fewer and the caller gave none,
  unless it reads as a place's name (two words or more with the number not last, such as "24
  Hour Sample", and no street address of its own); a row
  with the caller's own unit is still taken beside it. A distance the row opens with ("0.2 mi")
  is left out before the row is read; a whole number right before the address ("5M, 350 Example
  Ave") is an apartment, not a distance.
- A business listing at the address is the same place as the plain address.
- The town must be right: a shared district or region name does not make two towns one. A row
  that names no town does not confirm the caller's place. In the US, Canada and Australia, a row
  that leaves out only the state, with the caller's town spelled the same, in a part of its own or
  right after the caller's street, and no postcode or other country shown, is taken; the risk left
  is a same-named town in another state on a site that shows neither state nor postcode. It is
  asked about when it also leaves out the caller's postcode (a place's listing by name, street and
  town included), when the same address is listed in another state
  beside it, or when the caller's town is spelled another way (`town_spelled_differently`). A US
  ZIP code confirms or contradicts a state
  by its first three digits. A state comes from its own part or code, or is written after the town
  with no comma ("Exampleton IL", "Exampleton New York"), never from a town named like a state
  ("Washington, PA" is not "Washington, DC").
- A country the caller gives or writes is never crossed: a row that shows another country is another
  place ("Paris, TX 75460" is not in France; "London, ON N6A 1A1" is not in the United Kingdom), and
  so is, for a caller in the US, Canada, the UK or Australia, a row whose postcode of digits is not of
  that country's form ("Vancouver 98660" is not in Canada).
- Spellings that mean the same thing agree: abbreviations, diacritics, case and punctuation,
  component order (number before or after the street), joined or split compound names, native
  digits, Japanese block addresses, transliteration, and a one-letter typo in a long name; an
  English ordinal word and its number ("Fifth Ave" and "5th Ave"); a house number's letter with a
  space or without ("12 A Example St" and "12A Example St").

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
are compared with their romanizations by consonant sounds. Korean road names ("예제대로1418번길",
"Yeje-daero 1418beon-gil") are read as one word in both scripts. A Japanese place name whose
reading is not built from its characters' listed readings gets `ambiguous` or `none`, never a
guess. Cities whose names share a reading ("福州" and "抚州", both "Fuzhou") ask unless a
province confirms the one meant.

## Places named without a house number

When the caller gives a place's name and no house number ("Example Park, Exampleton, CA"), the
name decides:

- a row whose first part is the same name, word for word (Street for St), is the place, whatever
  street follows the name, when its area fits the caller's: the same town, the same state, province
  or region (a name and its code are one region: "Illinois" and "IL"; a city that is a region of
  its own, such as "台中市" for "Taichung", counts as both), the same postcode and the same country;
- a row in another town, region or country, or under another postcode, is another place; so is a
  row on another street than one the caller typed ("Example Cafe, Oak St, ...");
- a row that does not state the caller's town or street asks (`place_unconfirmed`), and so does a
  row that adds a place the caller did not name when no country is given ("Exampleton, CA" may be
  in California or in Canada);
- a row that holds the caller's words in order with others added before or between them ("The
  Example Park", or a venue under its full name) is `ambiguous`: it may be the place;
- a row that starts with the name and goes on ("Example Park Ferry Terminal") is another place;
- a site's control is never a place: a row that holds the caller's whole text and otherwise only
  control words ("See more results for ...", "Search "..."", "... (see more)"). A row that adds a
  name to the caller's text ("Old Town Example Park") is a row like any other;
- same-name rows on two streets or in two towns (two branches of a chain) are two places: the
  answer asks (`several_places`).

A town typed alone ("Munich") is compared as a town, in its other languages too ("München").

A place's name with a postcode alone ("Example Center, 94000") is the name with the postcode as
its area. Rows of the very same name in one town are one place, except that a row the site
categorized beside one it did not (a pin dropped under the venue's own name) asks
(`several_places`); `prefer_on_read` then names the categorized row as below. `pick` takes no
categories, so a picker that knows which rows the site shows as venues should use `pick_explained`.

When a row has exactly the caller's name and another holds the name in a longer form (a pin dropped
under the name next to the venue under its official name), the answer is `ambiguous`. A row may say
that the site shows it with a category of its own (`Row { text, categorized: true }`, a venue type
such as "Convention center"). When the caller gave a town or postcode, every such row fits it,
exactly one of them is categorized and that one holds it in full, `prefer_on_read` names it: a picker reading the
site may take it, and a write still asks. So does a categorized longer-name row that holds the
caller's area in full when the exact name appears only outside it. With no area from the caller,
a read asks as a write does. The longer row is preferred only when its name ends with the caller's name and the
words it adds before it are not compass, age or size words: "Jane Q. Example Convention Center" for
"Example Center" may be preferred, but "The Example Park Hotel", "Jane Q. Example Park Suites" and
"North Example Park" may be other places named after the caller's, so the answer stays `ambiguous`
with no preference: a write asks and a read takes neither.

A picker around PlaceMatch decides what to do with `none` and `ambiguous` (ask in a write, refuse or
read on in a read). `Matcher::is_control`, `Matcher::shows_typed_text`, `Matcher::in_area` and
`Matcher::same_but_postcode` (in JavaScript `isControl`, `showsTypedText`, `inArea` and
`sameButPostcode`) answer the questions such a picker asks. `same_but_postcode` accepts another
postcode only for the caller's house, street and town in the same state, the same districts and
one postal area (the postcodes differ only in their last two characters); in a city that repeats
street names across districts (Berlin, Hamburg, London, Vienna, Budapest, Prague) both sides must
name the same district; `tests/place-names.jsonl` holds the cases, with the
picker's answer for each mode beside PlaceMatch's own.

## Accuracy

Measured on real addresses held out by region (see [bench/real](bench/real)): whole states,
provinces, source datasets and some entire countries were never used while developing the
matcher. The libpostal baseline uses libpostal v1.1.4's parser and near-duplicate checks with
the same policy.

Held-out split: 76,722 cases from 62 countries, 11 scripts. Cells are accuracy / wrong-pick rate.

| | PlaceMatch | libpostal baseline | both agree |
|---|---|---|---|
| all cases | 92.0% / 0.32% | 75.9% / 1.87% | 74.6% / 0.03% |
| one candidate (`pair`) | 93.0% / 0.12% | 79.3% / 0.44% | 77.4% / 0.00% |
| 2 to 8 candidates (`pick`) | 90.7% / 0.58% | 71.3% / 3.81% | 70.7% / 0.06% |
| government and open-data addresses | 96.0% / 0.05% | 82.6% / 1.28% | 80.7% / 0.00% |
| OpenStreetMap addresses and listings | 90.4% / 0.42% | 73.2% / 2.11% | 72.1% / 0.04% |
| Latin script | 94.5% / 0.32% | 81.1% / 1.34% | 79.1% / 0.03% |
| other scripts | 87.5% / 0.31% | 66.6% / 2.83% | 66.6% / 0.04% |
| no commas | 84.4% / 0.28% | 76.0% / 1.89% | 70.2% / 0.03% |
| transliterated | 58.5% / 0.25% | 35.2% / 0.25% | 35.1% / 0.00% |

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
const e = m.pickExplained("example center, exampleton, ny", [
  "Example Center, Exampleton, NY, USA",
  { text: "Jane Q. Example Convention Center, 429 Sample Ave, Exampleton, NY 10001, USA", categorized: true },
], { country: "US" });
// e.outcome === "ambiguous", e.reasons == ["place_name_variant"], e.prefer_on_read === 1
```

`pickExplained` returns the fields above in snake case (`prefer_on_read`, `cross_script`); `options`
may be left out or `null`. `pick` and `compare` are there too. Every call takes a query and rows of
at most 512 characters each (JavaScript string length) and at most 100 rows (`LIMITS`); longer input
throws a `RangeError` and is never cut. Within the limits, every text is read as at most 16 comma
parts and 256 characters (a Han, kana or Hangul character counts twice): a longer one first loses
middle parts that hold no number and no unit word, then keeps its first parts and its last ones. A
row that loses a number or unit word that way is asked about, never matched. More than 16 different
rows of one place are asked about rather than compared with each other, so a page of long rows
answers quickly. `@pomerado/placematch/loader` exports `load(bytes)` for other
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
