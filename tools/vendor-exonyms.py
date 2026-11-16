#!/usr/bin/env python3
"""Build data/extra/exonyms-wikidata.txt from Wikidata city labels (CC0).

Usage: python3 -I tools/vendor-exonyms.py cities.csv > crates/placematch/data/extra/exonyms-wikidata.txt

cities.csv is the CSV result of this query at https://query.wikidata.org (cities of 150,000 people or
more and their labels in Latin-script languages):

  SELECT ?city ?lang ?label WHERE {
    ?city wdt:P31/wdt:P279* wd:Q515 ; wdt:P1082 ?pop . FILTER(?pop >= 150000)
    ?city rdfs:label ?label . BIND(LANG(?label) AS ?lang)
    FILTER(?lang IN ("en","de","fr","es","it","pt","nl","pl","cs","sv","da","no","fi","hu","ro","tr",
      "ca","hr","sk","sl","lt","lv","et","id","ms","vi","sq","is","ga","eu","gl"))
  }

Each output line lists one city's names, English first. A name shared by two cities is left out.
"""
import csv, sys, unicodedata, collections

def key(s):
    s = unicodedata.normalize("NFKD", s.lower())
    s = "".join(c for c in s if not unicodedata.combining(c))
    for a, b in (("ß", "ss"), ("ł", "l"), ("ø", "o"), ("æ", "ae"), ("đ", "d"), ("ı", "i")):
        s = s.replace(a, b)
    return "".join(c for c in s if c.isalnum())

names = collections.defaultdict(dict)  # city -> {key: label}
english = {}
for row in csv.DictReader(open(sys.argv[1], encoding="utf-8")):
    label = row["label"].strip()
    if any(ch in label for ch in "(),/") or not label.isascii() and not all(unicodedata.category(c)[0] in "LZPM" for c in label):
        continue
    k = key(label)
    if len(k) < 4 or not k.isascii():
        continue
    names[row["city"]].setdefault(k, label)
    if row["lang"] == "en":
        english[row["city"]] = k
owners = collections.Counter(k for ks in names.values() for k in ks)
out = []
for city, ks in names.items():
    keep = [k for k in ks if owners[k] == 1]
    if len(keep) < 2 or city not in english or english[city] not in keep:
        continue
    canon = english[city]
    out.append("|".join([ks[canon]] + sorted(ks[k] for k in keep if k != canon)).lower())
print("# City names in other languages, English first, from Wikidata (CC0); see tools/vendor-exonyms.py")
for line in sorted(out):
    print(line)
