"""Collect bundled dependency notices from the locked Cargo dependency graph."""
import json
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parent.parent
metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--locked", "--offline", "--format-version", "1",
     "--filter-platform", "x86_64-pc-windows-msvc"], cwd=root, text=True, encoding="utf-8"))
sections = ["Scribe Seek — third-party licenses\n\n"
            "This file is embedded in the application (Licenses button).\n"
            "Dependency source code is available at the repositories listed below.\n"
            "Symphonia is used under MPL-2.0; its original source is unmodified.\n"
            "Build-time dependencies are included for completeness.\n"]
for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
    if package["name"] == "scribe-seek":
        continue
    directory = Path(package["manifest_path"]).parent
    candidates = set()
    for path in directory.rglob("*"):
        if path.is_file() and len(path.relative_to(directory).parts) <= 5:
            name = path.name.lower()
            if name.startswith(("license", "licence", "copying", "notice", "ofl", "ufl")) and path.suffix not in (".rs", ".py"):
                candidates.add(path)
    if package.get("source") is None:
        # Local crates may inherit their license from the workspace root.
        for parent in directory.parents:
            license_path = parent / "LICENSE"
            if license_path.is_file():
                candidates.add(license_path)
                break
            if parent == root.parent:
                break
    source = package.get('repository') or (
        'Local workspace package: ' + package['name'] if package.get('source') is None
        else 'https://crates.io/crates/' + package['name'])
    sections.append(f"\n{'=' * 72}\n{package['name']} {package['version']}\n"
                    f"SPDX: {package.get('license') or 'see license below'}\n"
                    f"Source: {source}\n")
    seen = set()
    for path in sorted(candidates):
        text = "\n".join(line.rstrip() for line in path.read_text(encoding="utf-8", errors="replace").splitlines()).strip()
        if text and text not in seen:
            label = path.relative_to(directory).as_posix() if path.is_relative_to(directory) else 'Workspace LICENSE'
            sections.append(f"\n--- {label} ---\n{text}\n")
            seen.add(text)
    if not seen:
        sections.append(f"\nSee https://crates.io/crates/{package['name']}/{package['version']} for license text.\n")
(root / "THIRD_PARTY_LICENSES.txt").write_text("".join(sections), encoding="utf-8")
print(f"Collected notices for {len(metadata['packages']) - 1} packages")
