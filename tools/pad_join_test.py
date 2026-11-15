#!/usr/bin/env python3
"""The Pages phone page must join a game over iroh from Chromium and WebKit (iOS browsers are all
WebKit). pad-host stands in for the game; the page is served from a local copy of the site
(pad/site.sh). Then every phone screen is laid out on phone and iPad viewports with their safe-area
insets: nothing may leave the safe area, clip its text, or be a touch target under 56 px.

Adapted from Gobble Grand Prix's tools/pad_join_test.py (https://github.com/dsktd/gobble-grand-prix,
Unlicense).

    pad_join_test.py <pad-host binary> <site dir> [screenshot dir]

PAD_ENGINES picks engines (default "chromium webkit"); PAD_CHROMIUM_CHANNEL the Chromium channel
(default "chromium", the full browser; "" for the headless shell).
"""

import asyncio
import functools
import http.server
import json
import os
import pathlib
import subprocess
import sys
import threading

from playwright.async_api import async_playwright

ENGINES = os.environ.get("PAD_ENGINES", "chromium webkit").split()
CHANNEL = os.environ.get("PAD_CHROMIUM_CHANNEL", "chromium")


def launch(p, engine):
    # Full Chromium: the headless shell fails web fonts that the first paint asks for
    extra = {"channel": CHANNEL} if engine == "chromium" and CHANNEL else {}
    return getattr(p, engine).launch(**extra)


async def join(p, engine, url):
    browser = await launch(p, engine)
    page = await browser.new_page()
    errors = []
    page.on("console", lambda m: m.type == "error" and errors.append(m.text))
    await page.goto(url)
    await page.click("#go")
    try:
        await page.wait_for_function("document.getElementById('cover').hidden", timeout=20_000)
        ok = True
    except Exception:
        ok = False
    status = await page.text_content("#status")
    await browser.close()
    return ok, status, errors


# name: viewport and safe-area insets (top, right, bottom, left), as in Gobble Grand Prix
DEVICES = {
    "iphone-se": ((375, 667), (20, 0, 0, 0)),
    "iphone-p": ((393, 852), (59, 0, 34, 0)),
    "iphone-l": ((852, 393), (0, 59, 21, 59)),
    "android-p": ((412, 915), (24, 0, 16, 0)),
    "ipad-air-p": ((820, 1180), (24, 0, 20, 0)),
    "ipad-air-l": ((1180, 820), (24, 0, 20, 0)),
    "ipad-split-s": ((375, 1024), (24, 0, 20, 0)),
}
HIT = 56
LONG = "Wwwwwwwwwwww"
ME = {"name": LONG, "colour": "#ff2d55", "score": 12340, "place": 1, "lead": True}
PROMPT = "Which of these countries is the smallest by area, counting only land and inland water bodies today?"
CHOICES = ["Saint Vincent and the Grenadines", "Democratic Republic of the Congo", "São Tomé and Príncipe", "Wwwwwwwwwwwwwwwwwwwwwwwwwwwwwwww"]
TIMER = {"left_ms": 9000, "limit_ms": 20000}


def view(screen, timer=None, error=None, **fields):
    return {"t": "view", "id": 1, "seq": 3, "me": ME, "error": error, "screen": screen, "timer": timer, **fields}


SCREENS = [
    ("01-join", ""),
    ("02-connecting", "cover('Connecting through the internet...', true)"),
    ("03-disconnected", "closed()"),
    ("04-lobby", view("lobby", players=[{"name": LONG, "colour": "#fff", "connected": i % 3 > 0} for i in range(8)], few=False)),
    ("05-wait", view("wait", line="Eyes on the TV")),
    ("06-category", view("category", TIMER, options=["Big & Small", "Summer Games", "Capitals"], voted=None)),
    ("07-choice", view("choice", TIMER, prompt=PROMPT, choices=CHOICES)),
    ("08-answered", view("answered", choice=3, text=CHOICES[3])),
    ("09-write", view("write", TIMER, "Too close to the truth! Try another fake.", prompt="The capital of Saint Vincent and the Grenadines is ____.")),
    ("10-written", view("written", text="W" * 40)),
    ("11-vote", view("vote", TIMER, prompt=PROMPT, options=[{"i": i, "text": "W" * 40} for i in range(8)])),
    ("12-voted", view("voted", text="W" * 40)),
    ("13-result", view("result", correct=True, gained=2340, streak=5, fooled=3, bluff=True)),
    ("14-slow", view("result", correct=None, gained=0, streak=0, fooled=0, bluff=False)),
    ("15-scores", view("scores", gap=12340, above=LONG)),
    ("16-podium", view("podium")),
]

CHECK = """([t, r, b, l, hit, fonts]) => {
  const W = innerWidth, H = innerHeight, bad = [];
  const name = (el) => el.id || `${el.tagName.toLowerCase()}.${el.className || ""} "${el.textContent.trim().slice(0, 20)}"`;
  const scroller = (el) => el.closest(".safe");
  for (const el of document.body.querySelectorAll("*")) {
    if (el.classList.contains("safe") || el.tagName === "SCRIPT" || el.tagName === "STYLE") continue;
    const box = el.getBoundingClientRect(), style = getComputedStyle(el);
    if (!box.width || !box.height || style.visibility === "hidden" || el.closest("[hidden]")) continue;
    // Screens scroll inside their safe box, so only the sides and the top are fixed edges
    if (box.left < l - 0.5 || box.right > W - r + 0.5 || (box.top < t - 0.5 && scroller(el).scrollTop === 0))
      bad.push(`${name(el)} outside the safe area: ${[box.left, box.top, box.right, box.bottom].map(Math.round)}`);
    if (el.matches("button, input")) {
      if (box.width < hit - 0.5 || box.height < hit - 0.5) bad.push(`${name(el)} target ${Math.round(box.width)}x${Math.round(box.height)}`);
      if (el.scrollWidth > el.clientWidth + 1 || el.scrollHeight > el.clientHeight + 1) bad.push(`${name(el)} clips its text`);
    }
    if (el.scrollWidth > el.clientWidth + 1 && style.overflow !== "visible" && !el.classList.contains("grow") && el.tagName !== "INPUT")
      bad.push(`${name(el)} overflows sideways`);
  }
  // A face no element uses may stay unloaded (WebKit); only a file that failed counts
  if (fonts) for (const face of document.fonts) if (face.status === "error") bad.push(`font ${face.family} ${face.weight} failed`);
  return bad;
}"""


async def layout(p, engine, url, shots):
    browser = await launch(p, engine)
    failures = []
    for device, ((w, h), insets) in DEVICES.items():
        page = await browser.new_page(viewport={"width": w, "height": h}, has_touch=True)
        page.on("dialog", lambda d: d.accept())
        for screen, script in SCREENS:
            await page.goto(url)
            t, r, b, l = insets
            await page.add_style_tag(content=f":root{{--sa-t:{t}px;--sa-r:{r}px;--sa-b:{b}px;--sa-l:{l}px}}")
            if isinstance(script, dict):
                script = f"opened('LAN'); receive({json.dumps(json.dumps(script))})"
            if script:
                await page.evaluate(f"() => {{ {script} }}")
            await page.evaluate("Promise.all(['20px Lilita', '400 20px Atkinson', '700 20px Atkinson'].map((f) => document.fonts.load(f).catch(() => {})))")
            if shots:
                await page.screenshot(path=str(shots / f"{screen}-{device}-{engine}.png"))
            for problem in await page.evaluate(CHECK, [t, r, b, l, HIT, engine != "chromium" or bool(CHANNEL)]):
                failures.append(f"{engine} {device} {screen}: {problem}")
        await page.close()
    await browser.close()
    return failures


async def main(host_bin, site, shots=None):
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=site)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    host = subprocess.Popen([host_bin, "120"], stdout=subprocess.PIPE, text=True)
    line = host.stdout.readline().split()
    assert line[:1] == ["id"], f"pad-host: {line}"
    url = f"http://127.0.0.1:{server.server_port}/pad/#g={line[1]}&pin=1234"
    # No lan= here: with a local page the phone page moves to it after 10 s of stalling, which
    # would leave the page mid-join. The join must succeed over iroh by itself.
    failed = False
    async with async_playwright() as p:
        for engine in ENGINES:
            ok, status, errors = await join(p, engine, url)
            print(f"{engine}: {'joined' if ok else 'FAILED'} ({status})")
            for error in errors[:5]:
                print(f"  {error}")
            # WebKit refuses relay hosts with trailing dots: a join that needs a fallback is broken
            failed |= not ok or any("TLS" in error for error in errors)
        host.kill()
        joins = sum(1 for l in host.stdout if l.startswith("joined"))
        print(f"pad-host saw {joins} joins")
        failed |= joins < len(ENGINES)
        if shots:
            shots = pathlib.Path(shots)
            shots.mkdir(parents=True, exist_ok=True)
        page = f"http://127.0.0.1:{server.server_port}/pad/#pin=1234&lan=127.0.0.1:7777"
        for engine in ENGINES:
            problems = await layout(p, engine, page, shots)
            print(f"{engine}: {len(SCREENS)} screens x {len(DEVICES)} viewports, {len(problems)} layout problems")
            for problem in problems:
                print(f"  {problem}")
            failed |= bool(problems)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main(*sys.argv[1:4])))
