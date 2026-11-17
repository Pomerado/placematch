# Matching policy and case format

PlaceMatch answers one question: is the address a caller wrote the same place as an
address a website offers? The benchmark's ground truth follows the rules below.

## Case format

One JSON object per line:

| field | meaning |
|---|---|
| `id` | case id |
| `country` | ISO 3166-1 alpha-2 code of the address, passed to the matcher as a hint |
| `script` | script of the caller's text (`Latn`, `Cyrl`, `Grek`, `Arab`, `Hebr`, `Hani`, `Jpan`, `Hang`, `Thai`, ...) |
| `task` | `pair` (one candidate: same place or not) or `pick` (2 to 8 candidates: choose one) |
| `query` | the caller's address text |
| `candidates` | the strings the website offered |
| `expected` | `match`, `none` or `ambiguous` |
| `accept` | indices that are a correct pick when `expected` is `match` |
| `roles` | for each candidate, why it is there (`true`, `neg_number`, `neg_city`, ...) |
| `tags` | how the case was made (`abbrev`, `translit`, `no_commas`, `neg_street_type`, ...) |

A matcher sees only `query`, `candidates` and `country`.

Prediction format, one per line: `{"id": "...", "outcome": "match" | "none" | "ambiguous", "index": <int or null>}`.

## Same place

Hard components. A contradiction on any of them means a different place:

- house number with its suffix (`12a`, `12 bis`; `10` and `100` differ);
- street name;
- street type when both sides give one (`Avenue X` and `Quai X` differ; `St` and `Street` agree);
- unit, flat or suite when both sides give one;
- town when both sides give one, and every place name must fit: a shared district does not
  make `Centro, Cidade Alta` the same as `Centro, Vila Baixa`;
- postcode when both sides give one;
- state, province or region when both sides give one (`IL` and `Missouri` differ; `Texas` and
  `TX` agree; a city that is a region of its own, such as `台中市`, counts as one);
- country when both sides give one;
- PO box number.

Soft components may be missing on either side: postcode, region, country, district, business or
building name. A row must still confirm where the caller's place is:

- a row that names no town where the caller gave one (`12 Example St, USA`, `Exampleton Mall, 1
  Example St` for `..., Exampleton, IL`) is not the caller's place unless the postcodes agree;
- where towns share names across states (the US, Canada, Australia), a row that leaves out only
  the state, with the caller's town spelled the same, in a part of its own or right after the
  caller's street, and no postcode or other country shown, is taken (`1 Example St, Exampleton`
  and `1 Example St Exampleton` for `..., Exampleton, IL`; a same-named town in another state, on
  a site that shows neither state nor postcode, is the risk left). It is asked about when it also
  leaves out the caller's postcode (`..., IL 60000`), unless it is a place's listing by name,
  street and town (`Example Building, 1 Example Bldg, Example City`); when the same street and
  number are listed in another state beside it (`..., Exampleton, WI`, `Kansas City 64105` for
  Kansas); and when the caller's town is spelled another way (`Exampletn`). A row that shows a postcode or
  country but no state does not confirm the caller's state; a US ZIP code confirms or contradicts
  a state by its first three digits (`Kansas City 64100` is in Missouri, not Kansas);
- with no country given or shown by the texts, a row that leaves out a place the caller named
  does not confirm it;
- a state comes only from the state part or code, or from a state written after the town with
  no comma (`Exampleton IL`, `Exampleton New York`, in the US, Canada and Australia), never from a
  town named like a state (`Washington, PA` is not `Washington, DC`; `Nevada, MO` is not
  `Nevada, IA`);
- a two-letter code after the town is the state, never the town (`Exampleton, NE`);
- a country the caller gives, or writes (`London, United Kingdom`), is never crossed: a row that
  shows another country is another place, on every path. A row shows a country by its name, a
  state or province with that country's postcode (`ON N6A 1A1`), a postcode form only that country
  writes, a state or province named in full after the town where only other countries have it
  (`London, Ontario` for the United Kingdom), or a two-letter state code after the town that only
  other countries use (`Paris, TX` for France). For a caller in the US, Canada, the United Kingdom
  or Australia, whose postcodes are written in one form, a row's postcode of digits in another
  form shows another country (`Vancouver 98660` for Canada; the United Kingdom's forms include
  Ireland's, and a ZIP+4 may be written without its hyphen). `UK`, `U.K.`, `US`, `U.S.` and `USA` written as
  the last part are the United Kingdom and the United States. A postcode or state does not tell
  Ireland, the United Kingdom and the Crown Dependencies apart (Northern Ireland is written both
  ways); a row that names the other of them as its country is still another place. When the row
  taken shows nothing of the caller's country and a row left out for its country would also have
  been the place, the answer asks;
- with no country given, a place name that is also a state's or province's (`Example Park, New
  York`) does not confirm a row in another town of that state: it may be the city.

A different compass word is a different street (`1 N Main St` and `1 S Main St`), never a typo, and
so is a compass word on one side only of the same street words (`1 Example St NE` and `1 Example
St`), even where the word is also a state code (`NE`). A letter the other side spells out is no
compass word (`Av S. Ejemplo` and `Avenida San Ejemplo`).

Two cities whose names share a reading in another script (`福州` and `抚州`, both `Fuzhou`) are
not told apart by the reading: a row in Latin script for a Han city whose reading several cities
share is `ambiguous` unless a province or region confirms it, and two Han names compare by their
characters.

A caller who names a place and gives no house number (`Example Park, Exampleton, CA`) is matched
by the name: a row whose first part is the same name word for word is the place, whatever street
follows, when its area fits (the same town, region, postcode and country), and when it is on the
street the caller typed, if any. A row in another town, region or country is another place. A row
that does not state the caller's town or street is `ambiguous`, and so is a row that adds a place
the caller did not name when no country is given (`CA` may be California or Canada). Same-name rows
on two streets or in two towns (two branches) are two places: `ambiguous`. A row with the caller's
words in order and others added before or between them is `ambiguous`; a row that starts with the
name and goes on (`Example Park Ferry Terminal`) is another place. A site control is never a place:
a row that holds the caller's whole text and otherwise only control words (`See more results for
...`, `Search "..."`); a row that adds a name (`Old Town Example Park`) is not a control.

A candidate that adds a unit the caller did not give is not a match. In a `pick` case where the
only candidates that fit add a unit, the answer is `ambiguous`: ask the caller which unit.

Business listings count: `Cafe Luna, 10 Main St, Springfield` is the same place as
`10 Main St, Springfield`.

Equivalent spellings: abbreviations, diacritics and their folds, case, punctuation, component
order, native digits, Japanese `1丁目2番3号` and `1-2-3`, joined or split compound street names,
transliteration between a native script and Latin, and a one-letter typo in a caller's street or
town name of six or more letters (never a changed digit, and never in a town typed alone:
`Homburg` is not `Hamburg`).

With no country given, PlaceMatch reads the texts by the one country they show (see
`Matcher::infer_country`): a country's name, a state or province with its postcode, or a postcode
only one country writes. When they show none, or two, the texts are read with no country, which
asks more often; when rows of two countries would each be the place, read by their own country
(`10 Main St, Vancouver` beside `..., Vancouver, WA` and `..., Vancouver, BC`), the answer asks. A
country hint is an ISO code in any case, `UK` is `GB`, and an unknown code is no hint.

## Outcomes for `pick`

- `match`: exactly one distinct place among the candidates is the caller's place (several strings
  for that one place may exist; any of them is accepted);
- `ambiguous`: two or more distinct places each fit, or only unit listings fit;
- `none`: no candidate is the caller's place.

## Scoring

A wrong pick is a `match` answer whose index is not in `accept`. It is the costly error: an agent
would act on the wrong place. Accuracy counts every outcome. `bench/real/score_real.py` reports both
per task, case kind, perturbation, script, source and country, and breaks wrong picks down by the
role of the listing picked.
