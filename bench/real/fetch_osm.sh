#!/bin/sh
# fetch_osm.sh <out_dir> <python> < extracts.txt
# Each input line: "<geofabrik path> <country code or gcc>", e.g. "europe/greece GR".
# Downloads one Geofabrik extract at a time, samples address records, deletes the extract.
set -u
OUT=$1; PY=$2; HERE=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$OUT/pbf"
while read -r path cc; do
  [ -z "$path" ] && continue
  name=$(basename "$path")
  grep -qx "done $name" "$OUT/done.txt" 2>/dev/null && continue
  f="$OUT/pbf/$name.osm.pbf"
  if curl -sfL -m 1800 -o "$f" "https://download.geofabrik.de/$path-latest.osm.pbf"; then
    nice -n 19 "$PY" -I "$HERE/osm_extract.py" "$f" "$name" "$cc" "$OUT" 2>> "$OUT/run.log" && echo "done $name" >> "$OUT/done.txt"
  else
    echo "download failed: $name" >> "$OUT/run.log"
  fi
  rm -f "$f"
done
