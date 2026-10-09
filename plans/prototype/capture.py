"""Captures moneyball's real screens for the flow board. Drives the installed binary in tmux, one scene at a time,
on a throwaway copy of the workspace, and turns each terminal into cell data (screens.js) that index.html renders
cell-exact in a light and a dark terminal palette. Then screenshots every frame (frames/<theme>/<scene>.jpg) for
the zoomed-out board.

Run with a Python that has Playwright (the nocode-ai venv):
    ../../../nocode-ai/venv/bin/python capture.py              # every scene, then screenshots
    ../../../nocode-ai/venv/bin/python capture.py home brief   # just these scenes
    ../../../nocode-ai/venv/bin/python capture.py --shots      # screenshots only (after a renderer change)
    ../../../nocode-ai/venv/bin/python capture.py --no-shots   # cells only

Never touches the real workspace. Network: fetch and crm fetch are never captured live. /brief and /funnel hand
the numbers to the configured LLM for commentary, so those scenes (llm=True) make one real model call each: they
are skipped, keeping their last capture, unless you pass --llm.
    ../../../nocode-ai/venv/bin/python capture.py --llm brief  # recapture one LLM scene on purpose
"""

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
WORKSPACE = Path.home() / "DEV/moneyball-test/moneyball-data"
MONEYBALL = Path.home() / ".cargo/bin/moneyball"
TMUX = "/nix/store/hrcgphnidfh5zjzmgki3myz1y0cp9z5w-tmux-3.7c/bin/tmux"
COLS, ROWS = 140, 42
SOCKET = "mbboard"
FIRST_RUN_DIR = "/Users/Kailash/DEV/fincity-ads"  # what a first-run frame shows as its folder

# - one scene: how to reach the screen in the real binary. kind "tui" runs moneyball in the pane; "shell" runs a
#   prompt and types `cmd` into it. Steps: ("type", text) types literally, ("key", name) presses a tmux key,
#   ("wait_for", text) waits until the pane shows it, ("sleep", seconds). `home` "empty" = a fresh user with no
#   secrets and no config; `cwd` "empty" = a folder with no workspace. A TUI scene gets its own empty sessions dir
#   (so no frame lists the capture's own sessions) unless `sessions` is "workspace".
SCENES = [
    # A - first run
    dict(id="firstrun", kind="tui", home="empty", cwd="empty", args=[], steps=[("wait_for", "MONEYBALL")]),
    dict(id="firstrun_meta", kind="tui", home="empty", cwd="empty", args=[],
         steps=[("wait_for", "MONEYBALL"), ("key", "Enter"), ("sleep", 1)]),
    # B - daily use, on the last day with full CRM data
    dict(id="home", kind="tui", args=["--date", "2026-08-12"], steps=[("wait_for", "ask moneyball")]),
    dict(id="palette", kind="tui", args=["--date", "2026-08-12"], steps=[("wait_for", "ask moneyball"), ("type", "/")]),
    dict(id="brief", kind="tui", llm=True, args=["--date", "2026-08-12"],
         steps=[("wait_for", "ask moneyball"), ("type", "/brief"), ("key", "Enter"), ("sleep", 2)]),
    dict(id="funnel", kind="tui", llm=True, args=["--date", "2026-08-12"],
         steps=[("wait_for", "ask moneyball"), ("type", "/funnel Disha Namma Mane"), ("key", "Enter"), ("sleep", 2)]),
    dict(id="crm", kind="tui", args=["--date", "2026-08-12"],
         steps=[("wait_for", "ask moneyball"), ("type", "/crm"), ("key", "Enter"), ("sleep", 1.5)]),
    dict(id="report", kind="tui", args=["--date", "2026-08-12"],
         steps=[("wait_for", "ask moneyball"), ("type", "/report"), ("key", "Enter"), ("wait_for", "open in browser")]),
    dict(id="help", kind="tui", args=["--date", "2026-08-12"],
         steps=[("wait_for", "ask moneyball"), ("type", "/help"), ("key", "Enter"), ("sleep", 1)]),
    # funnel with the model switched off on the copy: the deterministic table, no paid call
    dict(id="funnel_nollm", kind="tui", nollm=True, args=["--date", "2026-08-12"],
         steps=[("wait_for", "ask moneyball"), ("type", "/funnel Disha Namma Mane"), ("key", "Enter"), ("sleep", 2)]),
    # the verdict gate on the CRM-less snapshot, in the TUI (model off: no paid call)
    dict(id="funnel_nocrm_tui", kind="tui", nollm=True, args=[],
         steps=[("wait_for", "ask moneyball"), ("type", "/funnel Disha Namma Mane"), ("key", "Enter"), ("sleep", 2)]),
    dict(id="brief_nocrm_tui", kind="tui", nollm=True, args=[],
         steps=[("wait_for", "ask moneyball"), ("type", "/brief"), ("key", "Enter"), ("sleep", 2)]),
    dict(id="keychain", kind="tui", args=["--date", "2026-08-12"],
         steps=[("wait_for", "ask moneyball"), ("type", "/keychain"), ("key", "Enter"), ("sleep", 1)]),
    dict(id="snapshots", kind="tui", args=["--date", "2026-08-12"],
         steps=[("wait_for", "ask moneyball"), ("type", "/snapshot"), ("key", "Enter"), ("sleep", 1.5)]),
    dict(id="unknown", kind="tui", args=["--date", "2026-08-12"],
         steps=[("wait_for", "ask moneyball"), ("type", "/ledger"), ("key", "Enter"), ("sleep", 1)]),
    dict(id="setup_rerun", kind="tui", args=["--date", "2026-08-12"],
         steps=[("wait_for", "ask moneyball"), ("type", "/setup"), ("key", "Enter"), ("sleep", 1.5)]),
    # C - the advisor, replayed from a saved session (no LLM call)
    dict(id="resume", kind="tui", sessions="workspace", args=["--resume", "mb-20260928T181010Z-z95e"],
         steps=[("sleep", 3)]),
    # F - states the data puts the user in (latest snapshot has no CRM data)
    dict(id="nocrm_crm", kind="tui", args=[],
         steps=[("wait_for", "ask moneyball"), ("type", "/crm"), ("key", "Enter"), ("sleep", 1.5)]),
    dict(id="badarg", kind="tui", args=["--date", "2026-08-12"],
         steps=[("wait_for", "ask moneyball"), ("type", "/report week"), ("key", "Enter"), ("sleep", 1)]),
    # T - the shipped daily-refresh redesign, drawn by the binary (no --date: the copy's real state)
    dict(id="landing_live", kind="tui", args=[], steps=[("wait_for", "ask moneyball"), ("sleep", 1)]),
    dict(id="status_live", kind="tui", args=[],
         steps=[("wait_for", "ask moneyball"), ("type", "/status"), ("key", "Enter"), ("sleep", 1)]),
    dict(id="schedule_live", kind="tui", args=[],
         steps=[("wait_for", "ask moneyball"), ("type", "/schedule"), ("key", "Enter"), ("sleep", 1)]),
    # D - headless upkeep, in a shell
    dict(id="cli_crm_status", kind="shell", cmd="moneyball crm status"),
    dict(id="cli_report", kind="shell", cmd="moneyball report --date 2026-08-12"),
    dict(id="cli_fetch_notoken", kind="shell", home="empty", cmd="moneyball fetch"),
    dict(id="cli_debug_reports", kind="shell", cmd="moneyball debug --reports"),
    dict(id="cli_list", kind="shell", cmd="moneyball --list"),
    dict(id="cli_crm_connect", kind="shell", cmd="moneyball crm connect"),
    dict(id="cli_brief_nocrm", kind="shell", cmd="moneyball brief"),
    dict(id="cli_funnel_nocrm", kind="shell", cmd='moneyball funnel "Disha Namma Mane"'),
    dict(id="cli_funnel_ok", kind="shell", cmd='moneyball funnel "Disha Namma Mane" --date 2026-08-12'),
]


def tmux(*args: str, check: bool = True) -> str:
    out = subprocess.run([TMUX, "-L", SOCKET, *args], capture_output=True, text=True)
    if check and out.returncode != 0:
        raise RuntimeError(f"tmux {' '.join(args)}: {out.stderr.strip()}")
    return out.stdout


def pane(escapes: bool = False) -> str:
    return tmux("capture-pane", "-t", "cap", "-p", *(["-e"] if escapes else []))


def wait_for(text: str, timeout: float = 20) -> None:
    end = time.time() + timeout
    while time.time() < end:
        if text in pane():
            return
        time.sleep(0.2)
    raise TimeoutError(f"never saw {text!r}:\n{pane()}")


# - the screen stops changing (a spinner or a typing reply would keep it moving)
def settle(quiet: float = 0.6, timeout: float = 15) -> None:
    end = time.time() + timeout
    last, since = None, time.time()
    while time.time() < end:
        now = pane(escapes=True)
        if now != last:
            last, since = now, time.time()
        elif time.time() - since >= quiet:
            return
        time.sleep(0.15)


def run_scene(scene: dict, ws: Path, homes: dict[str, Path], empty_dir: Path) -> dict:
    home = homes[scene.get("home", "real")]
    cwd = empty_dir if scene.get("cwd") == "empty" else ws
    env = {"HOME": str(home), "PATH": f"{MONEYBALL.parent}:/usr/bin:/bin", "TERM": "xterm-256color",
           "LANG": "en_US.UTF-8", "BASH_SILENCE_DEPRECATION_WARNING": "1"}
    if scene["kind"] == "tui" and scene.get("sessions") != "workspace":
        sessions = Path(tempfile.mkdtemp(prefix="mb-sessions-"))
        env["MONEYBALL_SESSIONS_DIR"] = str(sessions)
    tmux("kill-server", check=False)
    config = ws / ".moneyball/config.json"
    saved = config.read_text()
    if scene.get("nollm"):
        cfg = json.loads(saved)
        for key in ("model_provider", "model"):
            cfg.pop(key, None)
        config.write_text(json.dumps(cfg))
    if scene["kind"] == "tui":
        command = " ".join([str(MONEYBALL), *scene["args"]])
    else:
        env["PS1"] = r"\[\e[38;5;8m\]moneyball-data\[\e[0m\] $ "
        command = "bash --noprofile --norc"
    env_args = [f"{k}={v}" for k, v in env.items()]
    tmux("new-session", "-d", "-s", "cap", "-x", str(COLS), "-y", str(ROWS), "-c", str(cwd),
         "env", "-i", *env_args, *command.split())
    if scene["kind"] == "shell":
        wait_for("$")
        tmux("send-keys", "-t", "cap", "-l", scene["cmd"])
        tmux("send-keys", "-t", "cap", "Enter")
        time.sleep(1)
    for step, value in scene.get("steps", []):
        if step == "type":
            tmux("send-keys", "-t", "cap", "-l", value)
        elif step == "key":
            tmux("send-keys", "-t", "cap", value)
        elif step == "wait_for":
            wait_for(value)
        elif step == "sleep":
            time.sleep(value)
    settle()
    raw = pane(escapes=True)
    tmux("kill-server", check=False)
    config.write_text(saved)
    # the copy's paths read as the user's own (macOS shows a temp dir through /private, so swap that form first)
    for fake, real in [(ws, str(WORKSPACE)), (empty_dir, FIRST_RUN_DIR), (homes["empty"], "~")]:
        for form in (str(fake.resolve()), str(fake)):
            raw = raw.replace(form, real)
    return {"kind": scene["kind"], "cwd": FIRST_RUN_DIR if scene.get("cwd") == "empty" else str(WORKSPACE),
            "cmd": scene.get("cmd") or "moneyball " + " ".join(scene.get("args", [])), "cols": COLS, "rows": ROWS,
            "lines": parse_ansi(raw)}


# ── ANSI to cells: each line is a list of runs [text, style]; style is space-separated classes
#    (f<n>/b<n> palette fg/bg, B bold, D dim, I italic, U underline, R reverse) or #rrggbb colours ──

SGR = re.compile(r"\x1b\[([0-9;]*)m")


def parse_ansi(raw: str) -> list:
    state: dict = {}
    lines = []
    for line in raw.rstrip("\n").split("\n"):
        runs: list = []
        pos = 0
        for m in SGR.finditer(line):
            if m.start() > pos:
                push(runs, line[pos:m.start()], state)
            apply_sgr(state, m.group(1))
            pos = m.end()
        if pos < len(line):
            push(runs, line[pos:], state)
        lines.append(runs)
    while lines and not "".join(t for t, _ in lines[-1]).strip():
        lines.pop()
    return lines


def push(runs: list, text: str, state: dict) -> None:
    text = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", text)  # anything that is not colour is not ours to draw
    if not text:
        return
    style = " ".join(f"{k}{v}" if k in ("f", "b") else k for k, v in sorted(state.items()) if v is not None)
    if runs and runs[-1][1] == style:
        runs[-1][0] += text
    else:
        runs.append([text, style])


def apply_sgr(state: dict, params: str) -> None:
    codes = [int(c) if c else 0 for c in params.split(";")] if params else [0]
    i = 0
    while i < len(codes):
        c = codes[i]
        if c == 0:
            state.clear()
        elif c in (1, 2, 3, 4, 7):
            state["BDIUR"[[1, 2, 3, 4, 7].index(c)]] = ""
        elif c == 22:
            state.pop("B", None), state.pop("D", None)
        elif c in (23, 24, 27):
            state.pop({23: "I", 24: "U", 27: "R"}[c], None)
        elif 30 <= c <= 37 or 90 <= c <= 97:
            state["f"] = c - 30 if c < 90 else c - 82
        elif 40 <= c <= 47 or 100 <= c <= 107:
            state["b"] = c - 40 if c < 100 else c - 92
        elif c == 39:
            state.pop("f", None)
        elif c == 49:
            state.pop("b", None)
        elif c in (38, 48) and i + 1 < len(codes):
            key = "f" if c == 38 else "b"
            if codes[i + 1] == 5 and i + 2 < len(codes):
                state[key] = codes[i + 2]
                i += 2
            elif codes[i + 1] == 2 and i + 4 < len(codes):
                state[key] = "#%02x%02x%02x" % tuple(codes[i + 2:i + 5])
                i += 4
        i += 1


# - a scratch dir whose real path is exactly as long as `real`, so the TUI wraps and truncates the frame's lines
#   exactly as it would for the user's own path (the path is swapped back in after)
def twin_dir(real: str, tag: str) -> Path:
    room = len(real) - len("/private/tmp/")
    name = (tag + "-" + "x" * room)[:room]
    path = Path("/tmp") / name
    if path.exists():
        shutil.rmtree(path)
    return path


def capture_scenes(only: list[str]) -> None:
    out_file = HERE / "screens.js"
    screens = json.loads(out_file.read_text().split("=", 1)[1].rstrip().rstrip(";")) if out_file.exists() else {}
    with tempfile.TemporaryDirectory(prefix="mb-capture-") as tmp:
        tmp = Path(tmp)
        ws = twin_dir(str(WORKSPACE), "mbcap-ws")
        shutil.copytree(WORKSPACE, ws, symlinks=True)
        homes = {"real": Path.home(), "empty": tmp / "home"}
        homes["empty"].mkdir()
        empty_dir = twin_dir(FIRST_RUN_DIR, "mbcap-new")
        empty_dir.mkdir()
        for scene in SCENES:
            if only and scene["id"] not in only:
                continue
            if scene.get("llm") and "--llm" not in sys.argv:
                print("skipped", scene["id"], "(calls the LLM; pass --llm to recapture)")
                continue
            screens[scene["id"]] = run_scene(scene, ws, homes, empty_dir)
            print("captured", scene["id"])
        write_reports(ws)
        shutil.rmtree(ws)
        shutil.rmtree(empty_dir)
    out_file.write_text("// Generated by capture.py from the real moneyball binary: one terminal per scene. Do not edit.\n"
                        f"const SCREENS = {json.dumps(screens, ensure_ascii=False)};\n")


# - the HTML reports the report frames show, generated by today's binary on the copy (never the stale files on
#   disk): the daily report with full CRM data, and the month-to-date one
REPORT_RUNS = {"daily": ["--date", "2026-08-12"], "month": ["--date", "2026-08-10", "--window", "9"],
               "nocrm": ["--date", "2026-09-28"]}


def write_reports(ws: Path) -> None:
    (HERE / "reports").mkdir(exist_ok=True)
    for name, args in REPORT_RUNS.items():
        out = subprocess.run([str(MONEYBALL), "report", *args], cwd=ws, capture_output=True, text=True, check=True).stdout
        path = Path(re.search(r"open in browser: (.+\.html)", out).group(1).strip())
        html = path.read_text()
        (HERE / "reports" / f"{name}.js").write_text(
            "// Generated by capture.py: an HTML report as today's binary writes it. Do not edit.\n"
            f"REPORTS[{json.dumps(name)}] = {json.dumps(html, ensure_ascii=False)};\n")
        print("report", name, path.name, f"{len(html) / 1e6:.1f} MB")


def screenshot(only: list[str]) -> None:
    from playwright.sync_api import sync_playwright

    with sync_playwright() as pw:
        browser = pw.chromium.launch()
        page = browser.new_page()
        page.goto(f"{(HERE / 'index.html').as_uri()}?still")
        scenes = [s for s in page.evaluate("FRAMES.map(f => f.scene)") if not only or s in only]
        page.close()
        context = browser.new_context(viewport={"width": 1440, "height": 900}, device_scale_factor=2)
        page = context.new_page()
        for theme in ("light", "dark"):
            out = HERE / "frames" / theme
            out.mkdir(parents=True, exist_ok=True)
            for scene in scenes:
                page.goto(f"{(HERE / 'index.html').as_uri()}?scene={scene}&still&theme={theme}")
                page.wait_for_load_state("networkidle")
                page.evaluate("document.fonts.ready")
                page.wait_for_function("window.SCENE_READY === true", timeout=15000)
                page.screenshot(path=str(out / f"{scene}.jpg"), type="jpeg", quality=88)
                print(f"{theme}/{scene}")
        browser.close()


if __name__ == "__main__":
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if "--shots" not in sys.argv:
        capture_scenes(args)
    if "--no-shots" not in sys.argv:
        screenshot(args)
