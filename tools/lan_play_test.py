#!/usr/bin/env python3
"""End to end: the real host (console build) and three headless phones on its LAN page play a
Short match to the podium. Phones tap the first choice they are offered and write random fakes.

    lan_play_test.py <hunch binary>
"""

import asyncio
import os
import random
import subprocess
import sys

from playwright.async_api import async_playwright

PLAYERS = 3
CHANNEL = os.environ.get("PAD_CHROMIUM_CHANNEL", "chromium")


async def bot(page, n, done):
    seen = set()
    while not done.is_set():
        await asyncio.sleep(0.3)
        view = await page.evaluate("typeof view === 'undefined' || !view ? null : {screen: view.screen, seq: view.seq}")
        if not view:
            continue
        seen.add(view["screen"])
        if view["screen"] == "write":
            await page.fill("#screen input", f"Fake {random.randint(100, 999)} {n}")
        buttons = page.locator("#screen button")
        if view["screen"] in ("category", "choice", "write", "vote") and await buttons.count():
            await buttons.nth(random.randrange(await buttons.count())).click()
        if view["screen"] == "lobby" and n == 0 and await buttons.count() and len(seen) == 1:
            await asyncio.sleep(1.0)
            await buttons.first.click()
        if view["screen"] == "podium":
            break
    return seen


async def main(binary):
    host = subprocess.Popen([binary], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
    url = None
    while url is None:
        line = host.stdout.readline()
        assert line, "host exited"
        if line.startswith("Same Wi-Fi: "):
            url = line.split(": ", 1)[1].strip()
    # Short match: Down to Length, Left from Normal to Short
    host.stdin.write("s\na\n")
    host.stdin.flush()
    lines = []

    def pump():
        for line in host.stdout:
            lines.append(line.rstrip())

    asyncio.get_running_loop().run_in_executor(None, pump)
    async with async_playwright() as p:
        extra = {"channel": CHANNEL} if CHANNEL else {}
        browser = await p.chromium.launch(**extra)
        pages = []
        for n in range(PLAYERS):
            context = await browser.new_context(ignore_https_errors=True, viewport={"width": 393, "height": 852})
            page = await context.new_page()
            await page.goto(url)
            await page.fill("#name", f"Bot{n}")
            await page.click("#go")
            pages.append(page)
        done = asyncio.Event()
        try:
            seen = await asyncio.wait_for(asyncio.gather(*(bot(page, n, done) for n, page in enumerate(pages))), 400)
        except TimeoutError:
            seen = None
        done.set()
        await browser.close()
    host.kill()
    print("\n".join(lines[-12:]))
    if seen is None:
        print("FAILED: no podium within the time limit")
        return 1
    want = {"lobby", "category", "choice", "result", "write", "vote", "scores", "podium"}
    for n, screens in enumerate(seen):
        missing = want - screens
        print(f"Bot{n}: saw {sorted(screens)}" + (f", MISSING {sorted(missing)}" if missing else ""))
        if missing:
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main(sys.argv[1])))
