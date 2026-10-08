"""Re-renders daily-refresh-build-prompt.html from its .md (the page renders the Markdown with marked at load).
    python3 plans/render_prompt.py
"""

import json
import re
from pathlib import Path

HERE = Path(__file__).resolve().parent
md = (HERE / "daily-refresh-build-prompt.md").read_text()
page = HERE / "daily-refresh-build-prompt.html"
html, n = re.subn(r"^const MD = .*;$", lambda _: f"const MD = {json.dumps(md)};", page.read_text(), count=1, flags=re.M)
assert n == 1, "no MD line in the page"
page.write_text(html)
print("rendered", page.name)
