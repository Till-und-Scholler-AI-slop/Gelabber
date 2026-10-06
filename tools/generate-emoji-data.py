"""Generate the shared picker/validation data from pinned Unicode sources."""
import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "shared/data/emoji-sources"
EXPECTED = {
    "emoji-test-18.0.txt": "8f3735cda1f92a779d78af67cf86066bb1f07143dc22f2ac29394d9bc57ab21a",
    "annotations-de-48.2.3.json": "bfd22f5226d874843a28b6efec01db7e47dfae4ed802eceab8b416550f09e3c2",
    "annotations-derived-de-48.2.3.json": "486a878923ef6d8303fd9b8f4472486c60575cb64a811a7dbef451bf064c4496",
    "LICENSE.txt": "e7a93b009565cfce55919a381437ac4db883e9da2126fa28b91d12732bc53d96",
}
for name, digest in EXPECTED.items():
    if hashlib.sha256((SOURCE / name).read_bytes()).hexdigest() != digest:
        raise SystemExit(f"Pinned source hash mismatch: {name}")

annotations = {}
for name, key in [("annotations-de-48.2.3.json", "annotations"), ("annotations-derived-de-48.2.3.json", "annotationsDerived")]:
    annotations.update(json.loads((SOURCE / name).read_text())[key]["annotations"])

groups = {
    "Smileys & Emotion": "Gesichter und Gefühle", "People & Body": "Menschen und Körper",
    "Animals & Nature": "Tiere und Natur", "Food & Drink": "Essen und Trinken",
    "Travel & Places": "Reisen und Orte", "Activities": "Aktivitäten", "Objects": "Dinge",
    "Symbols": "Symbole", "Flags": "Flaggen", "Component": "Bestandteile",
}
entries = []
group = ""
for line in (SOURCE / "emoji-test-18.0.txt").read_text().splitlines():
    if line.startswith("# group:"):
        group = groups[line.split(":", 1)[1].strip()]
    if "; fully-qualified" not in line:
        continue
    codepoints = line.split(";", 1)[0].split()
    emoji = "".join(chr(int(point, 16)) for point in codepoints)
    note = annotations.get(emoji) or annotations.get(emoji.replace("\ufe0f", "")) or {}
    fallback = line.split("#", 1)[1].strip().split(" ", 2)[2]
    entries.append({"emoji": emoji, "label": note.get("tts", [fallback])[0], "keywords": note.get("default", []), "group": group})

output = {"version": "18.0", "cldr": "48.2.3", "emoji": entries}
encoded = json.dumps(output, ensure_ascii=False, separators=(",", ":")) + "\n"
outputs = {
    ROOT / "shared/data/emoji-18.0.json": encoded.encode("utf-8"),
    ROOT / "web/src/messages/emoji-data.json": encoded.encode("utf-8"),
    ROOT / "web/public/emoji-license.txt": (SOURCE / "LICENSE.txt").read_bytes(),
}
if "--check" in sys.argv:
    for path, expected in outputs.items():
        if not path.exists() or path.read_bytes() != expected:
            raise SystemExit(f"Generated data drift: {path.relative_to(ROOT)}")
else:
    for path, expected in outputs.items():
        path.write_bytes(expected)
print(f"Verified {len(entries)} canonical Unicode emoji" if "--check" in sys.argv else f"Generated {len(entries)} canonical Unicode emoji")
