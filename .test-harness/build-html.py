#!/usr/bin/env python3
"""Generate .test-harness/desired-state/index.html from features.toml.

Run after editing features.toml:
    python3 .test-harness/build-html.py
"""

import sys
from pathlib import Path

try:
    import tomllib  # Python 3.11+
except ModuleNotFoundError:
    import tomli as tomllib  # type: ignore

REPO_ROOT = Path(__file__).parent.parent
TOML_PATH = REPO_ROOT / ".test-harness" / "features.toml"
HTML_PATH = REPO_ROOT / ".test-harness" / "desired-state" / "index.html"


def build_features_js(features):
    rows = []
    for f in features:
        rows.append(
            "  {{ id: {id!r}, area: {area!r}, name: {name!r},\n"
            "    expected: {expected!r},\n"
            "    screenshot: {screenshot!r},\n"
            "    status: {status!r},\n"
            "    notes: {notes!r} }}".format(
                id=f.get("id", ""),
                area=f.get("area", ""),
                name=f.get("name", ""),
                expected=f.get("expected", ""),
                screenshot=f.get("screenshot", ""),
                status=f.get("status", "pending"),
                notes=f.get("notes", ""),
            )
        )
    return ",\n".join(rows)


def main():
    if not TOML_PATH.exists():
        print(f"error: {TOML_PATH} not found", file=sys.stderr)
        return 1
    data = tomllib.loads(TOML_PATH.read_text())
    features = data.get("features", [])
    if not features:
        print("error: no [[features]] entries in features.toml", file=sys.stderr)
        return 1

    # Load template
    template_path = (
        REPO_ROOT / ".test-harness" / "desired-state" / "index.template.html"
    )
    if not template_path.exists():
        print(f"error: template not found at {template_path}", file=sys.stderr)
        return 1
    template = template_path.read_text()

    features_js = build_features_js(features)
    out = template.replace("__FEATURES_JS__", features_js)

    HTML_PATH.parent.mkdir(parents=True, exist_ok=True)
    HTML_PATH.write_text(out)
    print(f"wrote {HTML_PATH} ({len(features)} features)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
