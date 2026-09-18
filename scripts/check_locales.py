"""Validate bundled translations without network access or extra packages."""
import json
from pathlib import Path
import re

root = Path(__file__).resolve().parent.parent
codes = ("en", "ru", "fr", "de", "es", "it")
catalogs = {code: json.loads((root / "locales" / f"{code}.json").read_text(encoding="utf-8")) for code in codes}
reference = catalogs["en"]
for code, catalog in catalogs.items():
    assert catalog.keys() == reference.keys(), f"Missing or extra translation keys: {code}"
    for key, value in catalog.items():
        assert value.strip(), f"Empty translation: {code}/{key}"
        assert sorted(re.findall(r"\{([^}]+)\}", key)) == sorted(re.findall(r"\{([^}]+)\}", value)), f"Format arguments changed: {code}/{key}"
for source in (root / "src").glob("*.rs"):
    for match in re.finditer(r'\.(?:tr|format)\(\s*("(?:[^"\\]|\\.)*")', source.read_text(encoding="utf-8")):
        key = json.loads(match.group(1))
        assert key in reference, f"Unknown UI translation key: {source.name}/{key}"
print(f"Validated {len(codes)} locales, {len(reference)} messages and all static UI translation keys")
