#!/usr/bin/env python3
"""Drives the real launcher window through tauri-driver (W3C WebDriver) for
test-e2e.sh. Standard library only.

drive.py SCENARIO GAME_DIR STATE_DIR [SHOTS_DIR]

SCENARIO:
  patch    log in, install the stable patch, check the panel's link to the
           full pre-release notes (clicked through a stand-in xdg-open), switch
           to the pre-release "mods" (with the one-time confirmation), then back
           to Stable, checking the game folder after each step
  events   the event badges come from the account API's events/current and
           festa answers (STATE_DIR/events-current.json, festa.json); with
           that API down, from the game server's flags (Hunter Festa only);
           then the same in the PS4 style.
           Screenshots go to SHOTS_DIR (optional 4th argument)
  proton   Steam Deck: with no Wine (MEZEPORTA_WINE_CMD missing) and the
           installer's Proton command (MEZEPORTA_PROTON_CMD, a stand-in that
           records its calls in STATE_DIR/proton-calls.log), Start runs
           "proton run <meze-deps.exe> --stdin-b64" even when the setting says
           "portable"; with "proton" set and Dev Mode off, opening Settings
           keeps it and offers Proton
  update   the update bar: a tampered download is refused, then the signed one
           is installed and the launcher restarts into it
Prints "ok ..." per check; exits 1 at the first failure.

Env: NEXT_VERSION  the version the update offers (default 1.5.4-hv3);
     NEW_VERSION   if set, the header must read "release ver. NEW_VERSION";
     SHOTS         if set, a folder for screenshots of the character page
                   (default window size and a small one), of the open Patch
                   panel with the notes link, and of the Wine
                   Prefix setting (proton).
"""
import base64
import hashlib
import http.client
import json
import os
import sys
import time
import urllib.request

DRIVER = "http://127.0.0.1:4444"
NEXT_VERSION = os.environ.get("NEXT_VERSION", "1.5.4-hv3")
ELEMENT = "element-6066-11e4-a52e-4f735466cecc"


def fail(msg):
    print("FAIL " + msg, flush=True)
    sys.exit(1)


def ok(msg):
    print("ok   " + msg, flush=True)


def call(method, path, body=None, timeout=60, retries=3):
    """One WebDriver request. tauri-driver sometimes resets a proxied
    connection to WebKitWebDriver (seen on mhfz-dev, "Connection reset by
    peer" in its log, the launcher unaffected): such resets are retried."""
    data = None if body is None else json.dumps(body).encode()
    for attempt in range(retries + 1):
        req = urllib.request.Request(DRIVER + path, data=data, method=method,
                                     headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                return json.loads(resp.read() or b"{}").get("value")
        except urllib.error.HTTPError as e:
            text = e.read()[:300]
            if e.code >= 500 and b"connection" in text.lower() and attempt < retries:
                time.sleep(1)
                continue
            raise RuntimeError("%s %s: %s" % (method, path, text))
        except (ConnectionError, http.client.HTTPException, urllib.error.URLError):
            if attempt >= retries:
                raise
            time.sleep(1)


class Session:
    def __init__(self, app):
        caps = {"capabilities": {"alwaysMatch": {"tauri:options": {"application": app}}}}
        self.id = call("POST", "/session", caps, timeout=120)["sessionId"]

    def js(self, script, *args):
        return call("POST", "/session/%s/execute/sync" % self.id, {"script": script, "args": list(args)})

    def wait(self, script, what, timeout=60, *args):
        end = time.time() + timeout
        last = None
        while time.time() < end:
            try:
                last = self.js(script, *args)
            except RuntimeError as e:
                last = str(e)
                last = None if "no such" in last else last
            if last:
                return last
            time.sleep(0.5)
        fail("timed out waiting for %s (last: %r)" % (what, last))

    def screenshot(self, path):
        with open(path, "wb") as f:
            f.write(base64.b64decode(call("GET", "/session/%s/screenshot" % self.id)))

    def set_size(self, width, height):
        call("POST", "/session/%s/window/rect" % self.id, {"width": width, "height": height})

    def close(self):
        try:
            call("DELETE", "/session/" + self.id, timeout=20)
        except Exception:
            pass


# JavaScript helpers, run in the launcher page.
CLICK_TEXT = """
const [selector, text] = arguments;
const visible = (e) => !!(e.offsetWidth || e.offsetHeight || e.getClientRects().length);
for (const e of document.querySelectorAll(selector)) {
  if (visible(e) && !e.disabled && e.textContent.trim().startsWith(text)) { e.click(); return true; }
}
return false;
"""
TYPE = """
const [selector, value] = arguments;
const e = document.querySelector(selector);
if (!e) return false;
e.value = value; e.dispatchEvent(new Event('input', {bubbles: true}));
return true;
"""
TEXT_OF = """
const e = document.querySelector(arguments[0]);
return e ? e.textContent.trim() : null;
"""


def sha(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def same(a, b):
    return os.path.isfile(a) and os.path.isfile(b) and sha(a) == sha(b)


def login(s):
    s.wait("return !!document.querySelector('#username_input')", "the login form", 90)
    s.js(TYPE, "#username_input", "hunter")
    s.js(TYPE, "#password_input", "secret")
    time.sleep(0.5)
    s.wait(CLICK_TEXT.replace("e.textContent.trim().startsWith(text)", "true"), "the LOG IN button", 10,
           "[data-controller-node='login']", "")


def install_patch_dialog(s, what):
    s.wait(CLICK_TEXT, "the patch dialog's Install button (%s)" % what, 60, "button", "Install")
    ok("patch dialog shown and Install clicked (%s)" % what)


def chip(s, expect, what):
    text = s.wait("""const e = document.querySelector('.hv-chip');
                     return e && e.textContent.trim() === arguments[0] ? e.textContent.trim() : null;""",
                  "patch button '%s' (%s)" % (expect, what), 90, expect)
    ok("character page, patch button says '%s'" % text)


# The Patch button must be part of the launcher's own layout (so it scales with
# it) and must not cover anything shown there, such as the footer's CAPCOM logo.
CHIP_PLACE = """
const chip = document.querySelector('.hv-chip');
if (!chip) return 'no patch button';
const shown = (e) => {
  for (let n = e; n && n !== document.documentElement; n = n.parentElement) {
    const st = getComputedStyle(n);
    if (st.display === 'none' || st.visibility === 'hidden' || parseFloat(st.opacity) === 0) return false;
  }
  return true;
};
const r = chip.getBoundingClientRect();
const hit = (b) => r.left < b.right - 1 && b.left < r.right - 1 && r.top < b.bottom - 1 && b.top < r.bottom - 1;
const bad = [];
const content = document.querySelector('.launcher-content');
for (const e of content.querySelectorAll('img, button, a, input, [class*="footer-status"], [class*="footer-tag"]')) {
  if (e === chip || e.contains(chip) || chip.contains(e) || !shown(e)) continue;
  const b = e.getBoundingClientRect();
  if (b.width && b.height && hit(b)) {
    const what = e.tagName === 'IMG' ? (e.getAttribute('src') || '').split('/').pop() : e.textContent.trim().slice(0, 30);
    bad.push(e.tagName.toLowerCase() + ' ' + what);
  }
}
if (bad.length) return 'covers: ' + bad.join(', ');
if (!content.contains(chip)) return 'not part of the launcher layout';
if (r.left < 0 || r.top < 0 || r.right > innerWidth || r.bottom > innerHeight) return 'outside the window';
return 'ok';
"""

HEADER_VERSION = """
for (const e of document.querySelectorAll('.launcher-content *')) {
  if (e.children.length === 0 && e.textContent.trim().startsWith('release ver.')) return e.textContent.trim();
}
return null;
"""


def check_layout(s):
    """Patch button placement (and header version) at the default size and a small one."""
    shots = os.environ.get("SHOTS")
    time.sleep(2)  # let the footer's fade and the character card's fade-in finish
    size = s.js("return [window.innerWidth, window.innerHeight];")
    for label, (w, h) in (("default", size), ("small", (562, 300))):
        if label == "small":
            try:
                s.set_size(w, h)
            except RuntimeError as e:
                fail("could not resize the window: %s" % e)
            time.sleep(1.5)
            w, h = s.js("return [window.innerWidth, window.innerHeight];")
        if shots:
            s.screenshot(os.path.join(shots, "character-page-%s-%dx%d.png" % (label, w, h)))
        where = s.js(CHIP_PLACE)
        if where != "ok":
            fail("patch button at %dx%d: %s" % (w, h, where))
        ok("patch button at %dx%d: inside the launcher layout, covers nothing" % (w, h))
    s.set_size(*size)
    time.sleep(1.5)
    want = os.environ.get("NEW_VERSION")
    if want:
        got = s.js(HEADER_VERSION)
        if got != "release ver. " + want:
            fail("header says %r, expected 'release ver. %s'" % (got, want))
        ok("header says '%s'" % got)


def files_are(game, folder, extra_absent=()):
    for dirpath, _d, names in os.walk(folder):
        for n in names:
            src = os.path.join(dirpath, n)
            rel = os.path.relpath(src, folder)
            if not same(src, os.path.join(game, rel)):
                fail("%s in the game folder differs from %s" % (rel, folder))
    for rel in extra_absent:
        if os.path.exists(os.path.join(game, rel)):
            fail("%s is still in the game folder" % rel)
    if os.path.exists(os.path.join(game, "tmp")):
        fail("tmp/ left in the game folder")
    with open(os.path.join(game, "other.txt")) as f:
        if f.read().strip() != "keep":
            fail("a file no patch touches was changed")


def read_channel(state):
    with open(os.path.join(state, "channel")) as f:
        return f.read().strip()


def open_panel(s):
    s.wait("const e = document.querySelector('.hv-chip'); if (e) { e.click(); return true; } return false;",
           "the patch button", 30)
    s.wait("return !!document.querySelector('.hv-panel .hv-choices')", "the patch panel's choices", 30)


NOTES_URL = "https://frontier.hunsterverse.net/account.html#client-patch"
NOTES_LINK = """
for (const a of document.querySelectorAll('.hv-panel a'))
  if (a.textContent.trim() === 'Full pre-release notes on the website' && a.offsetWidth && a.offsetHeight) return true;
return false;
"""


def notes_link(s, state):
    """With a pre-release slot out, the open panel links to the full notes on
    the website. The click goes to the stand-in xdg-open (test-e2e.sh), which
    records the URL instead of starting a browser."""
    s.wait(NOTES_LINK, "the pre-release notes link in the panel", 10)
    ok("panel: 'Full pre-release notes on the website' link shown (pre-release mods is out)")
    shots = os.environ.get("SHOTS")
    if shots:
        path = os.path.join(shots, "patch-panel-notes-link.png")
        s.screenshot(path)
        ok("screenshot " + path)
    s.wait(CLICK_TEXT, "a click on the notes link", 10, ".hv-panel a", "Full pre-release notes")
    log = os.path.join(state, "browser-calls.log")
    end = time.time() + 15
    while time.time() < end and not os.path.exists(log):
        time.sleep(0.3)
    if not os.path.exists(log):
        fail("clicking the notes link opened nothing (no xdg-open call)")
    with open(log) as f:
        urls = f.read().split()
    if urls != [NOTES_URL]:
        fail("the notes link opened %r, expected %s" % (urls, NOTES_URL))
    err = s.js(TEXT_OF, ".hv-panel .hv-error")
    if err:
        fail("panel shows an error after the click: %r" % err)
    if not s.js("return !!document.querySelector('.hv-panel .hv-choices')"):
        fail("the panel closed or lost its choices after the click")
    ok("notes link opens %s in the browser; panel still open, no error" % NOTES_URL)


def scenario_patch(game, state):
    s = Session(os.path.join(game, "mezeporta-bin"))
    try:
        login(s)
        install_patch_dialog(s, "first login, stable")
        chip(s, "Patch: Stable v1.30", "after the stable patch")
        check_layout(s)
        files_are(game, os.path.join(state, "stable"))
        ok("game folder holds the stable patch, other files untouched")

        open_panel(s)
        facts = s.js(TEXT_OF, ".hv-panel .hv-facts")
        if "Stable v1.30" not in facts or "up to date" not in facts:
            fail("panel facts: %r" % facts)
        ok("panel: installed and served = Stable v1.30, up to date")
        notes_link(s, state)
        s.wait(CLICK_TEXT, "the mods choice", 10, ".hv-panel .hv-choices button", "Pre-release mods")
        s.wait("return !!document.querySelector('.hv-confirm')", "the pre-release explanation", 20)
        if read_channel(state) != "stable":
            fail("channel changed before the player confirmed")
        ok("first pre-release choice asks for confirmation first (account still stable)")
        s.wait(CLICK_TEXT, "I understand", 10, ".hv-confirm button", "I understand")
        install_patch_dialog(s, "switch to mods")
        chip(s, "Patch: Pre-release mods v1.31-pre2", "after switching to mods")
        if read_channel(state) != "pre:mods":
            fail("account channel is %s" % read_channel(state))
        files_are(game, os.path.join(state, "slots", "mods"))
        ok("account on pre:mods (API), game folder holds the mods patch (hvmods.dll added)")

        open_panel(s)
        s.wait(CLICK_TEXT, "the Stable choice", 10, ".hv-panel .hv-choices button", "Stable")
        install_patch_dialog(s, "back to stable")
        chip(s, "Patch: Stable v1.30", "after switching back")
        if read_channel(state) != "stable":
            fail("account channel is %s" % read_channel(state))
        files_are(game, os.path.join(state, "stable"), extra_absent=["hvmods.dll"])
        ok("back on Stable: stable files restored, hvmods.dll removed, no backups left over")
        leftovers = [os.path.join(d, n) for d, _x, ns in os.walk(game) for n in ns if n.endswith(".mezeb")]
        stable_backups = ["mhfo-hd.dll.mezeb", os.path.join("dat", "a.pac.mezeb")]
        unexpected = [p for p in leftovers if os.path.relpath(p, game) not in stable_backups]
        if unexpected:
            fail("unexpected backups: %s" % unexpected)
        ok("only the stable patch's own backups of the original files remain")
    finally:
        s.close()


BADGE_TITLES = """
return [...document.querySelectorAll('.classic-event-badge-button, .ps4-event-badge-button')].map((e) => e.title);
"""
EVENT_PANEL = """
for (const e of document.querySelectorAll('.eventpaneltextC, .eventpaneltext'))
  if (e.querySelector('.eventpaneltextLabel')?.textContent.trim() === arguments[0]) return e.innerText;
return null;
"""
# The fixture's events (e2e/events-current.json, festa.json), in the launcher's order.
HV_BADGES = ["Hunter Festa", "Speedrun Cup", "VS Bracket", "Event campaigns",
             "MezFes", "Sky Corridor", "Raviente", "Hunting Road vote"]


def screenshot(s, shots, name):
    if not shots:
        return
    import base64
    os.makedirs(shots, exist_ok=True)
    path = os.path.join(shots, name)
    with open(path, "wb") as f:
        f.write(base64.b64decode(call("GET", "/session/%s/screenshot" % s.id)))
    ok("screenshot " + path)


def character_page_badges(s, expect, what):
    """Log in (installing a patch if one is offered) and wait for the badges."""
    login(s)
    end = time.time() + 90
    titles = None
    while time.time() < end:
        s.js(CLICK_TEXT, "button", "Install")
        titles = s.js(BADGE_TITLES)
        if titles == expect:
            # The badges pop in one by one: let them settle.
            time.sleep(1.5)
            return titles
        time.sleep(0.5)
    fail("%s: badges %r, expected %r" % (what, titles, expect))


def open_badge(s, title):
    s.js("""for (const e of document.querySelectorAll('.classic-event-badge-button, .ps4-event-badge-button'))
              if (e.title === arguments[0]) { e.click(); return true; }
            return false;""", title)
    text = s.wait(EVENT_PANEL, "the %s panel" % title, 10, title)
    time.sleep(0.5)  # the panel's pop-in transition
    return text


def scenario_events(game, state, shots=None):
    exe = os.path.join(game, "mezeporta-bin")
    s = Session(exe)
    try:
        character_page_badges(s, HV_BADGES, "with the events API")
        ok("badges from the API: %s" % ", ".join(HV_BADGES))
        screenshot(s, shots, "1-badges.png")
        text = open_badge(s, "Hunter Festa")
        if "Entry and vote" not in text:
            fail("Hunter Festa panel: %r" % text)
        ok("Hunter Festa panel names the phase (Entry and vote)")
        screenshot(s, shots, "2-hunter-festa.png")
        text = open_badge(s, "Speedrun Cup")
        if "Speedrun Cup: Test Zinogre" not in text:
            fail("Speedrun Cup panel: %r" % text)
        ok("Speedrun Cup panel names the quest")
        screenshot(s, shots, "3-speedrun-cup.png")
    finally:
        s.close()

    open(os.path.join(state, "events-down"), "w").close()
    s = Session(exe)
    try:
        character_page_badges(s, ["Hunter Festa"], "with the events API down")
        ok("events API down: the game server's flags are used (Hunter Festa only)")
        screenshot(s, shots, "4-api-down-fallback.png")
    finally:
        s.close()
        os.remove(os.path.join(state, "events-down"))

    # The PS4 style (style 3 in the launcher's store, Mezeporta/config.json);
    # afterwards the style the run started with (test-e2e.sh STYLE).
    before = set_style(game, 3)
    s = Session(exe)
    try:
        character_page_badges(s, HV_BADGES, "PS4 style")
        ok("PS4 style: the same badges")
        screenshot(s, shots, "5-ps4-badges.png")
        open_badge(s, "VS Bracket")
        screenshot(s, shots, "6-ps4-vs-bracket.png")
    finally:
        s.close()
        set_style(game, before)


def set_wine_mode(game, mode):
    path = os.path.join(game, "Mezeporta", "config.json")
    with open(path) as f:
        cfg = json.load(f)
    cfg["launcher_prefs"]["winePrefixMode"] = mode
    with open(path, "w") as f:
        json.dump(cfg, f, indent=2)


def wine_mode(game):
    with open(os.path.join(game, "Mezeporta", "config.json")) as f:
        return json.load(f)["launcher_prefs"]["winePrefixMode"]


LOG_TEXT = "return document.body.innerText;"


def start_through_proton(s, state, what):
    """Start the character; the stand-in Proton must get the game helper and
    its settings on stdin (the launcher then closes)."""
    calls = os.path.join(state, "proton-calls.log")
    before = os.path.getsize(calls) if os.path.exists(calls) else 0
    end = time.time() + 90
    while time.time() < end:
        s.js(CLICK_TEXT, "button", "Install")  # a patch dialog, if one is offered
        if s.js(CLICK_TEXT.replace("e.textContent.trim().startsWith(text)", "true"),
                "[data-controller-node='character-start']", ""):
            break
        time.sleep(0.5)
    else:
        s.screenshot(os.path.join(state, "proton-no-start.png"))
        fail("%s: no Start button (screenshot proton-no-start.png)" % what)
    end = time.time() + 60
    text = ""
    while time.time() < end:
        if os.path.exists(calls):
            with open(calls) as f:
                f.seek(before)
                text = f.read()
            if "END" in text:
                break
        time.sleep(0.5)
    else:
        try:
            s.screenshot(os.path.join(state, "proton-not-called.png"))
            page = s.js(LOG_TEXT)
        except Exception:
            page = "(window gone)"
        fail("%s: Proton was not called (calls %r; page says %r)" % (what, text, page[-400:] if page else page))
    args = [l[5:] for l in text.splitlines() if l.startswith("ARGS ")]
    if len(args) != 1 or not args[0].startswith("run ") or not args[0].endswith("meze-deps.exe --stdin-b64"):
        fail("%s: Proton calls %r" % (what, args))
    if "COMPAT " + os.path.join(state, "compat") not in text:
        fail("%s: STEAM_COMPAT_DATA_PATH not passed: %r" % (what, text))
    stdin = [l[6:] for l in text.splitlines() if l.startswith("STDIN ")][0]
    cfg = json.loads(base64.b64decode(stdin))
    if cfg.get("char_id") != 7:
        fail("%s: game settings on stdin for character %r" % (what, cfg.get("char_id")))
    ok("%s: %s, the game settings on stdin (character 7)" % (what, args[0]))


def scenario_proton(game, state):
    exe = os.path.join(game, "mezeporta-bin")
    # An older launcher reset the Deck's "proton" to "portable"; no Wine here.
    before = wine_mode(game)
    set_wine_mode(game, "portable")
    s = Session(exe)
    try:
        login(s)
        start_through_proton(s, state, "setting 'portable', no Wine")
    finally:
        s.close()

    set_wine_mode(game, "proton")
    s = Session(exe)
    try:
        login(s)
        s.wait("return !!document.querySelector(\"[data-controller-node='character-start']\")", "the character page", 90)
        s.wait(CLICK_TEXT.replace("e.textContent.trim().startsWith(text)", "true"), "the Settings button", 10,
               "[data-controller-node='settings']", "")
        s.wait(CLICK_TEXT.replace("e.textContent.trim().startsWith(text)", "true"), "the Advanced tab", 10,
               "[data-controller-node='settings-nav-advanced']", "")
        label = s.wait("""const e = document.querySelector("[data-controller-node='settings-wine-prefix-mode']");
                          return e ? e.textContent.trim() : null;""", "the Wine Prefix setting", 20)
        options = s.js("""return [...document.querySelectorAll('.settings-dropdown-option')]
                            .map((e) => e.textContent.trim());""")
        time.sleep(2)  # an old watcher reset it at once; give any save time to land
        if label != "Proton" or "Proton" not in options or wine_mode(game) != "proton":
            fail("Dev Mode off: setting shows %r, options %r, saved %r" % (label, options, wine_mode(game)))
        ok("Dev Mode off: Settings shows Wine Prefix 'Proton' (offered), still saved as proton")
        if os.environ.get("SHOTS"):
            s.screenshot(os.path.join(os.environ["SHOTS"], "settings-wine-prefix-proton.png"))
        # Back to the character page: the settings page's "Go back" (classic:
        # also the Settings button, which says "Go back" there).
        end = time.time() + 20
        while time.time() < end:
            if s.js("return !!document.querySelector(\"[data-controller-node='character-start']\")"):
                break
            s.js(CLICK_TEXT, "[data-controller-node='settings-nav-back'], [data-controller-node='settings']",
                 "Go back")
            time.sleep(1)
        else:
            s.screenshot(os.path.join(state, "proton-settings-back.png"))
            fail("could not leave Settings (screenshot proton-settings-back.png)")
        start_through_proton(s, state, "setting 'proton'")
        if wine_mode(game) != "proton":
            fail("winePrefixMode is now %r" % wine_mode(game))
    finally:
        s.close()
        set_wine_mode(game, before)


def set_style(game, style):
    """Sets the launcher style; returns the previous one (1, classic, if unset)."""
    path = os.path.join(game, "Mezeporta", "config.json")
    with open(path) as f:
        cfg = json.load(f)
    before = cfg.get("style", 1)
    cfg["style"] = style
    with open(path, "w") as f:
        json.dump(cfg, f, indent=2)
    return before


def launcher_pids(exe):
    pids = []
    for pid in os.listdir("/proc"):
        if not pid.isdigit():
            continue
        try:
            target = os.readlink("/proc/%s/exe" % pid)
        except OSError:
            continue
        if target.replace(" (deleted)", "") == exe:
            pids.append(int(pid))
    return pids


def scenario_update(game, state):
    exe = os.path.join(game, "mezeporta-bin")
    release = os.path.join(state, "release")
    good = os.path.join(release, "mezeporta-bin")
    before = sha(exe)
    # Serve a tampered binary first: the signature in latest.json is for the good one.
    os.rename(good, good + ".good")
    with open(good + ".good", "rb") as f:
        data = bytearray(f.read())
    data[len(data) // 2] ^= 0xFF
    with open(good, "wb") as f:
        f.write(data)
    s = Session(exe)
    try:
        s.wait("const e = document.querySelector('.hv-update-bar'); return e ? e.textContent : null;",
               "the update bar", 60)
        text = s.js(TEXT_OF, ".hv-update-bar strong")
        if text != "Launcher update " + NEXT_VERSION:
            fail("update bar says %r" % text)
        ok("update bar offers %s, %s" % (NEXT_VERSION, s.js(TEXT_OF, ".hv-update-bar span").strip("() ")))
        pids = launcher_pids(exe)
        if len(pids) != 1:
            fail("expected one launcher process, found %s" % pids)
        s.wait(CLICK_TEXT, "Update and restart", 10, ".hv-update-bar button", "Update and restart")
        err = s.wait("const e = document.querySelector('.hv-update-bar .hv-error'); return e ? e.textContent : null;",
                     "the refusal of the tampered file", 60)
        if "signature" not in err or sha(exe) != before or os.path.exists(exe + ".new"):
            fail("tampered update: %r, binary changed: %s" % (err, sha(exe) != before))
        ok("tampered download refused (%s), launcher file unchanged" % err.strip())

        os.replace(good + ".good", good)
        s.js(CLICK_TEXT, ".hv-update-bar button", "Update and restart")
        end = time.time() + 120
        while time.time() < end and not same(exe, good):
            time.sleep(0.5)
        if not same(exe, good):
            fail("the launcher file was not replaced")
        ok("signed update installed: the launcher file is now the %s build" % NEXT_VERSION)
        # The new binary removes <exe>.old at start-up: proof that it runs.
        end = time.time() + 60
        while time.time() < end and os.path.exists(exe + ".old"):
            time.sleep(0.5)
        if os.path.exists(exe + ".old"):
            fail("mezeporta-bin.old still there: the new launcher did not start")
        now = launcher_pids(exe)
        if now != pids:
            fail("launcher processes before %s, after %s (exec should keep the PID)" % (pids, now))
        ok("restarted in place (same PID %d, exec), the new launcher started and removed the old file" % pids[0])
        # The new launcher's window loads and checks for updates again (now up to date).
        log = os.path.join(state, "requests.log")
        end = time.time() + 60
        while time.time() < end:
            with open(log) as f:
                if sum(" GET /latest.json 200" in line for line in f) >= 2:
                    break
            time.sleep(0.5)
        else:
            fail("the restarted launcher did not check latest.json again")
    finally:
        s.close()


def main():
    scenario, game, state = sys.argv[1:4]
    if scenario == "events":
        shots = os.path.abspath(sys.argv[4]) if len(sys.argv) > 4 else None
        return scenario_events(os.path.abspath(game), os.path.abspath(state), shots)
    {"patch": scenario_patch, "proton": scenario_proton,
     "update": scenario_update}[scenario](os.path.abspath(game), os.path.abspath(state))


if __name__ == "__main__":
    main()
