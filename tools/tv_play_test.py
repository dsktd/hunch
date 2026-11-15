#!/usr/bin/env python3
"""End to end on the TV page: the real host, its /tv page in a headless browser and two headless
phones on the LAN page play the first round. The host is steered only with keys on the TV page
(Short match, Start); phones tap a random choice. Passes when the TV has shown the lobby with a QR
code, the round intro, a category vote, a question, its reveal and the scores, and then round 2.

    tv_play_test.py <hunch binary> [screenshot dir]
"""

import asyncio
import os
import random
import re
import subprocess
import sys

from playwright.async_api import async_playwright

PLAYERS = 2
CHANNEL = os.environ.get("PAD_CHROMIUM_CHANNEL", "chromium")
TV_STATE = "tv ? {screen: tv.screen, seq: tv.seq, round: tv.round.n, answer: tv.answer ?? null, pin: tv.join.pin} : null"


async def phone(page, done):
    while not done.is_set():
        await asyncio.sleep(0.3)
        view = await page.evaluate("typeof view === 'undefined' || !view ? null : view.screen")
        buttons = page.locator("#screen button")
        if view in ("category", "choice") and await buttons.count():
            # Vote slowly: the round moves on once everyone has voted, and the TV page must be seen on
            # the category screen between its 0.2 s samples
            await asyncio.sleep(1.5)
            await buttons.nth(random.randrange(await buttons.count())).click()


async def main(binary, shots):
    env = dict(os.environ, HUNCH_TV_PORT="0")
    host = subprocess.Popen(
        [binary], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1, env=env
    )
    tv_url = lan = None
    while tv_url is None:
        line = host.stdout.readline()
        assert line, "host exited"
        if line.startswith("TV: "):
            tv_url = line[4:].strip()
    while lan is None:
        line = host.stderr.readline()
        assert line, "host exited"
        if m := re.search(r"phones on (https://\S+)", line):
            lan = m.group(1)
    loop = asyncio.get_running_loop()
    loop.run_in_executor(None, lambda: [None for _ in host.stdout])
    loop.run_in_executor(None, lambda: [None for _ in host.stderr])
    seen = set()
    async with async_playwright() as p:
        browser = await p.chromium.launch(**({"channel": CHANNEL} if CHANNEL else {}))
        tv = await browser.new_page(viewport={"width": 1280, "height": 800})
        errors = []
        tv.on("console", lambda m: m.type == "error" and errors.append(m.text))
        tv.on("pageerror", lambda e: errors.append(str(e)))
        await tv.goto(tv_url)
        await tv.wait_for_function("tv && tv.screen === 'lobby'", timeout=10_000)
        seen.add("lobby")
        pin = (await tv.evaluate(TV_STATE))["pin"]
        qr = await tv.evaluate("document.getElementById('qr').width")
        assert qr > 20, "no QR code on the lobby"
        phones = []
        for n in range(PLAYERS):
            context = await browser.new_context(ignore_https_errors=True, viewport={"width": 393, "height": 852})
            page = await context.new_page()
            await page.goto(f"{lan}/?pin={pin}")
            await page.fill("#name", f"Bot{n}")
            await page.click("#go")
            phones.append(page)
        await tv.wait_for_function(f"tv.players.length === {PLAYERS}", timeout=15_000)
        if shots:
            await tv.screenshot(path=f"{shots}/tv-lobby.png")
        # Short match: Down to Length, Left from Normal to Short; Up to Start, OK
        for key in ("ArrowDown", "ArrowLeft", "ArrowUp", "Enter"):
            await tv.keyboard.press(key)
            await asyncio.sleep(0.2)
        done = asyncio.Event()
        bots = [asyncio.create_task(phone(page, done)) for page in phones]
        deadline = loop.time() + 180
        last = None
        while loop.time() < deadline:
            state = await tv.evaluate(TV_STATE)
            name = state["screen"] + ("-reveal" if state["screen"] == "question" and state["answer"] is not None else "")
            if state["round"] == 2:
                seen.add("round2")
                break
            if name != last and shots:
                await tv.screenshot(path=f"{shots}/tv-{len(seen):02}-{name}.png")
            seen.add(name)
            last = name
            await asyncio.sleep(0.2)
        done.set()
        await asyncio.gather(*bots)
        await browser.close()
    host.kill()
    want = {"lobby", "intro", "category", "question", "question-reveal", "scores", "round2"}
    missing = want - seen
    print(f"TV saw {sorted(seen)}" + (f", MISSING {sorted(missing)}" if missing else ""))
    if errors:
        print("page errors:", *errors, sep="\n  ")
    return 1 if missing or errors else 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else None)))
