"""PreToolUse gate: editor UI edits need fresh, verified source evidence.

Reads the hook's tool JSON on stdin. Blocks Write/Edit/NotebookEdit on the
editor's UI files unless a matching record in `.claude/source-evidence/`
exists, was written in the last 30 minutes, and every quoted source anchor is
still present in the source checkout.
"""
from __future__ import annotations

import json
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RECORDS = ROOT / ".claude" / "source-evidence"
SOURCE = Path(r"D:\git-repos\yunari")
FRESH_SECONDS = 30 * 60
UI_PREFIXES = ("crates/editor/src/ui.rs", "crates/editor/src/ui/")


def deny(reason: str) -> None:
    print(json.dumps({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }
    }))
    raise SystemExit(0)


def main() -> None:
    try:
        payload = json.load(sys.stdin)
    except Exception as error:
        deny(f"source-evidence: cannot read hook input: {error}")
    path = payload.get("tool_input", {}).get("file_path", "")
    rel = Path(path).resolve().relative_to(ROOT).as_posix() if path else ""
    if not rel.startswith(UI_PREFIXES):
        return

    record = RECORDS / (Path(rel).name + ".json")
    if not record.is_file():
        deny(
            f"source-evidence: missing {record.relative_to(ROOT).as_posix()}. "
            "Read the source code for this surface first, then write a record "
            "with source files and verbatim anchors."
        )
    try:
        data = json.loads(record.read_text(encoding="utf-8"))
    except Exception as error:
        deny(f"source-evidence: {record.name} is not valid JSON: {error}")

    age = time.time() - record.stat().st_mtime
    if age > FRESH_SECONDS:
        deny(
            f"source-evidence: {record.name} is {int(age / 60)} minutes old. "
            "Re-read the source and refresh the record before editing."
        )
    files = data.get("files")
    if not isinstance(files, list) or not files:
        deny(f"source-evidence: {record.name} has no files list.")

    missing: list[str] = []
    for item in files:
        if not isinstance(item, dict):
            missing.append("malformed file entry")
            continue
        source_path = SOURCE / item.get("path", "")
        if not source_path.is_file():
            missing.append(f"missing source file {item.get('path')}")
            continue
        try:
            text = source_path.read_text(encoding="utf-8", errors="replace")
        except OSError as error:
            missing.append(f"unreadable source file {item.get('path')}: {error}")
            continue
        for anchor in item.get("anchors", []):
            if not isinstance(anchor, str) or anchor not in text:
                missing.append(f"{item.get('path')}: anchor not found: {anchor[:80]!r}")
    if missing:
        deny("source-evidence: stale anchors — " + "; ".join(missing))


if __name__ == "__main__":
    main()
