"""Re-renders every <name>-build-prompt.html from its .md (the page renders the Markdown with marked at load). A
new prompt gets the daily-refresh page as its template.
    python3 plans/render_prompt.py
"""

import json
import re
from pathlib import Path

HERE = Path(__file__).resolve().parent
TEMPLATE = HERE / "daily-refresh-build-prompt.html"

for md_path in sorted(HERE.glob("*-build-prompt.md")):
    page = md_path.with_suffix(".html")
    html = (page if page.exists() else TEMPLATE).read_text()
    title = md_path.read_text().splitlines()[0].lstrip("# ").strip()
    html, n = re.subn(r"^const MD = .*;$", lambda _: f"const MD = {json.dumps(md_path.read_text())};", html, count=1, flags=re.M)
    assert n == 1, f"no MD line in {page.name}"
    html = re.sub(r"<title>.*?</title>", f"<title>{title}</title>", html, count=1)
    html = re.sub(r"<!-- Generated from .*? -->", f"<!-- Generated from {md_path.name} by render_prompt.py; edit the .md, not this file. -->", html, count=1)
    page.write_text(html)
    print("rendered", page.name)
