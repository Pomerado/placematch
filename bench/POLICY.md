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
  make `Centro, Cidade Alta` the same as `Centro, Vila Baixa`; in the US, the same house number,
  street and five-digit ZIP code on both sides settle the place whatever town or neighborhood
  each side writes (`New York` and `Manhattan`); the site's one row that differs only by naming
  another town in the caller's state is asked about, and a read may take it when the caller gave
  no postcode or both give the same one;
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
  leaves out the caller's postcode (`..., IL 60000`, and `Example Building, 1 Example Bldg,
  Example City` for `..., Example City, CA 94000`); when the same street and
  number are listed in another state beside it (`..., Exampleton, WI`, `Kansas City 64105` for
  Kansas); and when the caller's town is spelled another way (`Exampletn`). A row that shows a postcode or
  country but no state does not confirm the caller's state; a US ZIP code confirms or contradicts
  a state by its first three digits (`Kansas City 64100` is in Missouri, not Kansas);
- with no country given or shown by the texts, a row that leaves out a place the caller named
  does not confirm it;
- a caller who gave only a postcode (`62701`) is that postcode's area: a row that opens with the
  same postcode and then names only a town, region or country (`62701 Exampleton, IL, USA`) is that
  area, and a row that names a street before the town is a house on that street, another place:
  a street with a type word (`62701 Example Street, Exampleton`; `62701, Example Trail, Sampleton,
  IL`) or a part followed by another town other than a county (`62701 Example, Sampleton, IL`); a
  town of two words right after the number (`62701 New Exampleton, IL`) is the area, and so is one
  that ends in a few place-like street types (`62701 Example Grove, IL`; Grove, Garden, Park,
  Valley, Bay, Spring, Beach, Hill, Heights, Village, Falls, Island, Harbor) unless it holds any
  other street type (`62701 Example Loop Spring Valley, IL` is a house), a type word with a word
  that is no type word after it (`62701 Example Grove Sampleton, IL` is a house), or a name then
  three or more of them, or four or more alone (`62701 Example Grove Spring Valley, IL` and `62701
  Spring Grove Spring Valley, IL` are houses);
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
compass word (`Av S. Ejemplo` and `Avenida San Ejemplo`). `St` right before the compass words that
end a street, the same words on both sides, is the other side's `Street` (`1 Example St NW` and `1
Example Street Northwest`). It is read so only when the pick or comparison as read before takes no
place, so an answer that took a place stays as it was, row and all.

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
...`, `Search "..."`); a row that adds a name (`Old Town Example Park`) is not a control. A dash
with a space on each side separates a row's name as a comma does only before an address (a house
number and street, a postcode, or a town the caller gave); otherwise it is part of the name
(`Example Stadium - Gate A` is another place). A part of a place named before the caller's name
(`Terminal 2 — Example Airport`) is that part. A transport place's three-letter code in parentheses
(`Example Airport (EXA)`) is not part of the name; codes that differ name two places. A row whose first part names a venue by a place-type word the caller did
not write is that venue, not a place in a town the caller typed alone.

A candidate that adds a unit the caller did not give is not a match. In a `pick` case where the
only candidates that fit add a unit, the answer is `ambiguous`: ask the caller which unit. A
picker reading the site may take the first of them that is the caller's place but for its unit
(`prefer_on_read`) when the caller wrote only an address and those rows, at most 16 different ones,
are one place no row puts in another state.

Where the number comes first, or no country is given, a short number alone in its own part right
after a typed street that opens with its house number is the unit (`123 Example Street, 4,
Exampleton`): up to four digits, not opening with 0, with at most one letter after them (`4`,
`4B`, `200`). `Apt 4`, `#4`, `Suite 4` and `4` are one unit; `5` is another. Where the street comes
before the number (`Calle Mayor, 4`), or the street opens with no number of its own (`5th Avenue,
725`), the number is the house number. A street whose name opens with a number (`16 Mile Road,
4500`, `17 Avenue SW, 1440`) is written the same way, so the row is read both ways and a place that
fits either reading is a match. The caller's own text keeps its one reading.

A short part before the address that may name a unit, whatever its word, is read as the row's
unit: a number with a letter or "F" (`7F`, `12B`, `7/F`), a symbol and a number (`№7`, `#7`),
one or two letters glued to a number (`U7`), or one word and a number (`Studio 12`, `No. 7`).
A number alone or with a letter (`7`, `7 B`, `7-B`, `U-7`) counts too. Another unit is not the
caller's. The caller's own unit number, or no unit from the caller, is at most asked about, also
beside a row that fits, since a place's name may look the same (`Sample 57, 12 Example St`). A
part the caller wrote too, or whose number is the row's postcode (a postal branch), is not read
this way, and a longer name holding the caller's number (`Studio 12 Hair`, `12 Hour Sample`) is a
place's name.

Any other leading part with a number that a row is compared without (`Lower Level 7`, `P.H. 7`,
`Sample Shop 3`) is at most asked about when its number is not the address's and the caller gave
a unit, or when it has three words or fewer and the caller gave no unit. A part with two words or
more and its number not last (`24 Hour Sample`, `7 Sample Store`) is a place's name and is taken
by its address, unless it is a street address of its own (`50 Example Street`, `50A Example
Street`, `No. 50 Example Street`) or may be another unit: words after a unit's number (`Block 7
East`, `Piso 3 Puerta B`) look the same, so for a caller with a unit every number in the part must
be that unit's (`24 Hour Sample` for Suite 24), and for a caller with none the part must open with
its number and name no side (`24 Hour Sample`, not `Tower 2 West` or `7 East Wing`). A row taken
this way is told apart from other rows by its address, but a row that is the caller's place as
written is preferred over it (`Sample Cafe, 12 Example St` for a caller who named Sample Cafe).
The caller's house number written as a part of its own (`7, Example St`) is not the caller writing
a row's leading `7`, unless the caller wrote that number twice (`60, Example Rd 60`). Beside a row with the
caller's own unit, that row is still taken. A distance a row opens with (`0.2 mi`, `350 ft`) is
left out before the row is read, so it is never the house number. A whole number with a unit of
length right before the address (`5M, 350 Example Ave`, `5 m, 12 Example St`) is read as a
possible unit, not a distance: a distance has a decimal or a place's name after it. A leading part
that is only the caller's own house number or postcode (`7, 7 Example St`) is not taken as text
the caller wrote, so it is read as a possible unit too.

A business whose name opens with a number (`7-Eleven`, `3rd Street Deli`, `99 Ranch Market`) is
not asked about as a unit beside a row with the caller's full street address (house number,
street, and the caller's town or postcode), for a caller who gave a town or postcode and no unit,
when every such row is a business: the site categorized it, or its leading part has a word of
three letters or more that the address dictionaries know only as a place's or a company's word, or
not at all, in the singular too (`Deli`, `Market`, `Eleven`; not `Bay`, `Back`, `Units` or
`Suites`), that is no building-part word they lack (`Wing`, `Booth`, `Dock`, `Upstairs`, `Condo`,
`Carport`, `Parking` and a few more), and the part names no side and is no part that may be a unit
whatever its word (`Bay 12`, `12 Bay`, `Condo 7 Rear`, `7 East Wing` and a bare `7` are still asked
about). The other rows are then decided without the businesses, and their match is taken when its
row states the caller's full street address and every business, read without its name, is the same
place as each row of that place and as each other business; the businesses are then among the rows
of the place. Any other answer (a business under another postcode or in another state than those
rows or than another business, or any reason the other rows ask) keeps the question as before.

Business listings count: `Cafe Luna, 10 Main St, Springfield` is the same place as
`10 Main St, Springfield`. So is a listing named after its own street address (`10 Main St Parking,
10 Main St, Springfield`) when the words after the address hold no number and no street, direction,
unit, level, entrance or house-number word; `10 Main St North, 10 Main St`, `10 Main St Apt 4, 10
Main St` and `10 AB, 10, Main St`, whose next part names no street, are not read that way. Among the rows of the caller's place, the address alone is taken over the
listings named at it.

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
