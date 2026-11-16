#!/bin/sh
# Copy the subset of libpostal dictionaries, libaddressinput country data and
# OpenCage state names that placematch embeds. Re-run when upstream data changes.
set -e
SRC=${1:?usage: vendor-data.sh <dir with libpostal/, aid/ and opencage/ checkouts>}
DST=$(dirname "$0")/../crates/placematch/data
D=$SRC/libpostal/resources/dictionaries
rm -rf "$DST/dict" && mkdir -p "$DST/dict"
TYPES="street_types directionals unit_types_numbered unit_types_standalone level_types_numbered level_types_standalone level_types_basement level_types_mezzanine post_office stopwords house_numbers number synonyms concatenated_suffixes_separable concatenated_suffixes_inseparable personal_titles qualifiers building_types company_types place_names entrances staircases no_number"
for lang in $(ls "$D"); do
  [ -d "$D/$lang" ] || continue
  [ "$lang" = all ] && continue
  for t in $TYPES; do
    f="$D/$lang/$t.txt"
    [ -f "$f" ] || continue
    mkdir -p "$DST/dict/$lang"; cp "$f" "$DST/dict/$lang/$t.txt"
  done
done
mkdir -p "$DST/aid"; cp "$SRC"/aid/*.json "$DST/aid/"
cp "$SRC/opencage/conf/state_codes.yaml" "$DST/state_codes.yaml"
cp "$SRC/opencage/conf/country2lang.yaml" "$DST/country2lang.yaml"
du -sh "$DST"
