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
- PO box number.

Soft components may be missing on either side: postcode, region, country, district, business or
building name.

A different compass word is a different street (`1 N Main St` and `1 S Main St`), never a typo.

A caller who names a place and gives no house number (`Example Park, Exampleton, CA`) is matched
by the name: a row whose first part is the same name word for word is the place, whatever street
follows, when its area fits; a row with the caller's words in order and others added before or
between them is `ambiguous`; a row that starts with the name and goes on (`Example Park Ferry
Terminal`) is another place; a row that shows the caller's whole text after words of its own
(`See more results for ...`) is a site control and never a place.

A candidate that adds a unit the caller did not give is not a match. In a `pick` case where the
only candidates that fit add a unit, the answer is `ambiguous`: ask the caller which unit.

Business listings count: `Cafe Luna, 10 Main St, Springfield` is the same place as
`10 Main St, Springfield`.

Equivalent spellings: abbreviations, diacritics and their folds, case, punctuation, component
order, native digits, Japanese `1丁目2番3号` and `1-2-3`, joined or split compound street names,
transliteration between a native script and Latin, and a one-letter typo in a caller's street or
town name of six or more letters (never a changed digit).

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
