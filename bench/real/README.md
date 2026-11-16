# Real-address benchmark generator

These scripts build the benchmark from real, openly licensed address records. The data
is downloaded when you run them and is not stored in this repository.

## Sources

| source | license | used for |
|---|---|---|
| [Overture Maps](https://overturemaps.org) addresses theme (OpenAddresses-derived and government sources) | per record; `ov_sample.py` keeps only open licenses (CC0, CC-BY, public domain, DL-DE, Etalab and similar) | street-clustered addresses |
| [OpenStreetMap](https://www.openstreetmap.org/copyright) extracts from [Geofabrik](https://download.geofabrik.de) | ODbL 1.0, (c) OpenStreetMap contributors | addresses, business listings, street names in other scripts |
| Overture Maps divisions theme | ODbL 1.0 | town names in other languages and scripts |
| [ziozzang/korean_address_en-ko](https://huggingface.co/datasets/ziozzang/korean_address_en-ko) | MIT | Korean road names with official romanizations |

## Steps

Use a fresh directory for the downloaded data, outside this repository, and run Python with `-I`.
`pyarrow`, `duckdb` and `osmium` (pyosmium) are needed.

```sh
DATA=/path/to/data
python -I ov_meta.py $DATA/ov_meta.json                    # row-group statistics per country
python -I ov_sample.py $DATA/ov_meta.json $DATA/overture.jsonl
python -I ov_divisions.py $DATA/divisions.jsonl
mkdir -p $DATA/hf_ko && curl -fL -o $DATA/hf_ko/gil.json \
  https://huggingface.co/datasets/ziozzang/korean_address_en-ko/resolve/main/gil.json
printf 'asia/thailand TH\neurope/greece GR\nasia/japan/kyushu JP\n' | ./fetch_osm.sh $DATA/osm_a "$(command -v python3)"
python -I build_cases.py $DATA $DATA/cases
```

`fetch_osm.sh` downloads one extract at a time and deletes it after sampling. Several runs can
share the work by writing to different `osm_*` directories.

## How cases are made

Each case starts from one real address. The caller's text and the site's listings are renderings
of real records with the changes callers and sites make: abbreviations, missing units or
postcodes, reordering, missing commas, typos, transliteration, native digits, and business names.
Distractors are real neighbours: the same street with another number, another street in the same
town, the same street name in another town, another postcode in the same town, other businesses.

The split is by source region. A whole region (a state, province, department, source dataset or
city) is either `tune` or `heldout`, and some countries are held out entirely, so held-out
addresses come from places the matcher was never tuned on.

`conventions.py` holds the caller-side abbreviations and per-country writing conventions. They come
from postal style guides and common usage, not from the matcher's own tables.

## Scoring

```sh
python -I score_real.py $DATA/cases/heldout.jsonl placematch=pm.pred libpostal=lp.pred
```

`bench/libpostal/` has the libpostal baseline: a Dockerfile for libpostal v1.1.4, `lpmatch.py`
(libpostal parsing plus near-duplicate checks, following the same policy) and `both_agree.py`,
which answers `match` only when both matchers pick the same listing.
