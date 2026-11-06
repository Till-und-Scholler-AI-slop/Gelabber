"""Checks what the native app took over on its first start after the Tauri
app had run the fixture page (page/index.html) on 127.0.0.1:8123.

    python3 -I check_import.py <config-dir>

<config-dir> is the app's config directory (desktop.json, settings.json,
native-state.json). Exit status 1 with the difference when a value is not
the page's. Standard library only.
"""

import json
import pathlib
import sys

ACCOUNT = "0f9a4c1e-8f1b-4f3e-9d55-2b7a1c3e5d01"
FRIEND = "6b0e2a77-3c44-4d0a-a1c2-9e8f7d6c5b4a"
CHANNEL = "c2d3e4f5-a6b7-4c8d-9e0f-1a2b3c4d5e6f"
GONE = "d4e5f6a7-b8c9-4dae-8f01-2b3c4d5e6f70"
SETTINGS = {
    "inputGain": 1.5,
    "voiceInput": "ptt",
    "pttKey": "F13",
    "sourceAudioShare": "off",
    "uiScale": "large",
    "compactRooms": True,
}


def main() -> None:
    config = pathlib.Path(sys.argv[1])
    problems = []
    settings = json.loads((config / "settings.json").read_text(encoding="utf-8"))
    for key, value in SETTINGS.items():
        if settings.get(key) != value:
            problems.append(f"settings.json {key}: {settings.get(key)!r}, not {value!r}")
    if "theme" in settings or "greeting" in settings:
        problems.append(f"settings.json took more than the device settings: {sorted(settings)}")
    state = json.loads((config / "native-state.json").read_text(encoding="utf-8"))
    account = state.get("accounts", {}).get(ACCOUNT, {})
    volume = account.get("volumes", {}).get(FRIEND)
    if volume != {"volume": 0.5, "muted": True}:
        problems.append(f"the friend's volume: {volume!r}")
    drafts = account.get("drafts", {})
    if drafts.get(CHANNEL, {}).get("text") != "bis gleich \U0001f44b":
        problems.append(f"the fresh draft: {drafts.get(CHANNEL)!r}")
    for channel, why in ((FRIEND, "expired"), (GONE, "deleted")):
        if channel in drafts:
            problems.append(f"the {why} draft was taken: {drafts[channel]!r}")
    if problems:
        sys.exit("import check failed:\n  " + "\n  ".join(problems))
    print(f"import OK: {len(settings)} device settings, the volume and the fresh draft")


if __name__ == "__main__":
    main()
