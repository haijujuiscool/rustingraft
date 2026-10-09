"""Search poly.pizza and list CC0 hits (scrapes the search page's embedded JSON).

Usage: python -I tools/assets/polypizza.py <query>...   -> lines: publicID | title | creator | uuid
Download: https://static.poly.pizza/<uuid>.glb (verify licence on https://poly.pizza/m/<publicID>).
"""
import json
import re
import sys
import urllib.parse
import urllib.request

UA = {"User-Agent": "Mozilla/5.0"}
HIT = re.compile(r'\{"id":\d+,"title":.*?"licence":"[^"]*"\}')


def search(query):
    url = "https://poly.pizza/search/" + urllib.parse.quote(query)
    html = urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=30).read().decode("utf8")
    hits = []
    for m in HIT.finditer(html):
        try:
            h = json.loads(m.group(0))
        except json.JSONDecodeError:
            continue
        if h.get("licence") != "CC0 1.0":
            continue
        uuid = h.get("previewUrl", "").rsplit("/", 1)[-1].rsplit(".", 1)[0]
        hits.append((h["publicID"], h["title"], h["creator"]["username"], uuid))
    return hits


if __name__ == "__main__":
    for q in sys.argv[1:]:
        print(f"## {q}")
        for row in dict.fromkeys(search(q)):
            print(" | ".join(row))
