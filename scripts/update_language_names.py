"""Vendor localized language names from Unicode CLDR; no network at runtime."""
import concurrent.futures
import json
from pathlib import Path
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
LOCALES = ["en", "ru", "fr", "de", "es", "it"]
SOURCE = "https://raw.githubusercontent.com/unicode-org/cldr-json/main/cldr-json/cldr-localenames-full/main/{locale}/languages.json"

def load(locale):
    with urllib.request.urlopen(SOURCE.format(locale=locale), timeout=30) as response:
        data = json.load(response)["main"][locale]["localeDisplayNames"]["languages"]
    # Whisper retains older ISO aliases for Javanese and Tagalog.
    data["jw"] = data["jv"]
    data["tl"] = data.get("tl", data["fil"])
    return locale, data

with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
    names = dict(pool.map(load, LOCALES))
output = {"source": SOURCE, "license": "Unicode-3.0", "languages": names}
(ROOT / "locales").mkdir(exist_ok=True)
(ROOT / "locales" / "languages.json").write_text(json.dumps(output, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
print(f"Saved language names for {len(names)} interface locales")
