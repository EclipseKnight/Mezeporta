#!/usr/bin/env python3
"""Drives the real launcher window through tauri-driver (W3C WebDriver) for
test-e2e.sh. Standard library only.

drive.py SCENARIO GAME_DIR STATE_DIR

SCENARIO:
  patch    log in, install the stable patch, switch to the pre-release "mods"
           (with the one-time confirmation), then back to Stable, checking the
           game folder after each step
  update   the update bar: a tampered download is refused, then the signed one
           is installed and the launcher restarts into it
Prints "ok ..." per check; exits 1 at the first failure.
"""
import hashlib
import http.client
import json
import os
import sys
import time
import urllib.request

DRIVER = "http://127.0.0.1:4444"
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


def scenario_patch(game, state):
    s = Session(os.path.join(game, "mezeporta-bin"))
    try:
        login(s)
        install_patch_dialog(s, "first login, stable")
        chip(s, "Patch: Stable v1.30", "after the stable patch")
        files_are(game, os.path.join(state, "stable"))
        ok("game folder holds the stable patch, other files untouched")

        open_panel(s)
        facts = s.js(TEXT_OF, ".hv-panel .hv-facts")
        if "Stable v1.30" not in facts or "up to date" not in facts:
            fail("panel facts: %r" % facts)
        ok("panel: installed and served = Stable v1.30, up to date")
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
        if text != "Launcher update 1.5.4-hv3":
            fail("update bar says %r" % text)
        ok("update bar offers 1.5.4-hv3 (running 1.5.4-hv2)")
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
        ok("signed update installed: the launcher file is now the hv3 build")
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
    {"patch": scenario_patch, "update": scenario_update}[scenario](os.path.abspath(game), os.path.abspath(state))


if __name__ == "__main__":
    main()
