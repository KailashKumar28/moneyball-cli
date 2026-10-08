"""Packs the flow board into ONE file to share: board.html with its scripts, every frame image, and the whole
prototype (scripts and the HTML reports) inline, so a colleague opens it without the folder. Zoomed in, frames are
still live; Copy for agent and the Build plan work as on disk. Run after capture.py, with a Python that has Pillow
(the nocode-ai venv):
    ../../../nocode-ai/venv/bin/python share.py
Writes plans/flow-board.html (gitignored: it carries client data).
"""

import base64
import io
import json
import re
from pathlib import Path

from PIL import Image

HERE = Path(__file__).resolve().parent
OUT = HERE.parent / "flow-board.html"
FRAME_WIDTH = 1200  # zoomed out a frame never shows bigger; zoomed in, the live frame takes over


def pack() -> None:
    frames = {f"{theme}/{shot.stem}": webp(shot) for theme in ("light", "dark")
              for shot in sorted((HERE / "frames" / theme).glob("*.jpg"))}
    prototype = prototype_page()
    packed = (f"<script>\nconst SHARE_PROTO = {script_json(prototype)};\n"
              f"const SHARE_FRAMES = {script_json(frames)};\n</script>\n")
    board = once((HERE / "board.html").read_text(), '<script src="tokens.js"></script>', packed + '<script src="tokens.js"></script>')
    board = inline_scripts(board, ("tokens.js", "flows.js", "build.js"))
    OUT.write_text(board)
    print(f"{OUT} {OUT.stat().st_size / 1e6:.1f} MB: {len(frames)} frame images, prototype {len(prototype) / 1e6:.1f} MB")


# - the prototype as one page: scripts inline and every report preloaded, since a page opened from a blob can't
#   load reports/<name>.js beside it
def prototype_page() -> str:
    page = (HERE / "index.html").read_text()
    reports = "".join((HERE / "reports" / f"{name}.js").read_text() for name in sorted(p.stem for p in (HERE / "reports").glob("*.js")))
    page = once(page, "<script>window.REPORTS = window.REPORTS || {};</script>",
                "<script>\nwindow.REPORTS = {};\n" + reports.replace("</script", "<\\/script") + "\n</script>")
    return inline_scripts(page, re.findall(r'<script src="([\w.]+\.js)"></script>', page))


def inline_scripts(page: str, names) -> str:
    for name in names:
        code = (HERE / name).read_text().replace("</script", "<\\/script")
        page = once(page, f'<script src="{name}"></script>', f"<script>\n{code}\n</script>")
    return page


def webp(path: Path) -> str:
    image = Image.open(path)
    if image.width > FRAME_WIDTH:
        image = image.resize((FRAME_WIDTH, round(image.height * FRAME_WIDTH / image.width)), Image.LANCZOS)
    buf = io.BytesIO()
    image.save(buf, "WEBP", quality=70)
    return "data:image/webp;base64," + base64.b64encode(buf.getvalue()).decode()


def script_json(value) -> str:
    return json.dumps(value, ensure_ascii=False).replace("</", "<\\/")


def once(text: str, old: str, new: str) -> str:
    assert text.count(old) == 1, f"expected one {old!r}"
    return text.replace(old, new)


if __name__ == "__main__":
    pack()
