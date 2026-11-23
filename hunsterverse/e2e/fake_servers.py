#!/usr/bin/env python3
"""Local stand-ins for the end-to-end test (test-e2e.sh): no real server,
account or game data.

fake_servers.py STATE_DIR GAME_PORT API_PORT RELEASE_PORT

- GAME_PORT: the wrapper and Erupe's launcher API as the launcher sees them.
  POST /v2/login answers like Erupe's endpoints.go: patchServer is "" (the
  wrapper's own stable /check) or http://127.0.0.1:GAME_PORT/prerelease/<slot>
  when the account's channel is pre:<slot> and that slot exists
  (prerelease.go patchServerFor). GET /check and /<path> serve STATE_DIR/stable,
  /prerelease/<slot>/check and /prerelease/<slot>/<path> serve
  STATE_DIR/slots/<slot>, in the wrapper's /check format (sha256, tab, /path;
  ETag = sha256 of the body; 304 on If-None-Match).
- API_PORT: the REST API calls the website makes (PatchChannelController):
  POST /api/auth/signin, GET and PUT /api/account/patch-channel (409 with
  confirmationRequired for the first pre-release of an account that is not a
  playtester yet; PUT stable drops the role).
  GET /api/public/events/current and /api/public/festa (the Events page's
  answers) serve STATE_DIR/events-current.json and festa.json; while
  STATE_DIR/events-down exists they answer 503.
- RELEASE_PORT: static files from STATE_DIR/release (latest.json, binaries).
- Stalled download (the cancel scenario): while STATE_DIR/stall exists and
  holds a request path, that file is sent only half (the full Content-Length
  announced), STATE_DIR/stalled is created, and the rest follows once
  STATE_DIR/stall is gone (or after 120 s), so the test can cancel mid-file.

Account: username "hunter", password "secret". The channel lives in
STATE_DIR/channel (one line), so the test can read and set it. Every request
is appended to STATE_DIR/requests.log ("<port> <method> <path> <status>").
Slot versions: STATE_DIR/slots/<slot>.version; stable version: STATE_DIR/stable.version.
"""
import hashlib
import http.server
import json
import os
import sys
import threading
import time

STATE, GAME_PORT, API_PORT, RELEASE_PORT = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
USER, PASSWORD, TOKEN = "hunter", "secret", "test-token-1"
CONFIRM = ("Pre-release patches play only on the Test land, and nothing is saved there. "
           "Choosing one gives you the Playtester role.")
lock = threading.Lock()


def path_of(*parts):
    return os.path.join(STATE, *parts)


def read(name, default=""):
    try:
        with open(path_of(name), encoding="utf-8") as f:
            return f.read().strip()
    except FileNotFoundError:
        return default


def write(name, value):
    with open(path_of(name), "w", encoding="utf-8") as f:
        f.write(value + "\n")


def slots():
    root = path_of("slots")
    if not os.path.isdir(root):
        return []
    return sorted(d for d in os.listdir(root) if os.path.isdir(os.path.join(root, d)))


def manifest(folder):
    lines = []
    for dirpath, _dirs, files in os.walk(folder):
        for name in files:
            full = os.path.join(dirpath, name)
            rel = os.path.relpath(full, folder).replace(os.sep, "/")
            with open(full, "rb") as f:
                lines.append("%s\t/%s\n" % (hashlib.sha256(f.read()).hexdigest(), rel))
    body = "".join(sorted(lines)).encode()
    return body, '"%s"' % hashlib.sha256(body).hexdigest()


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def answer(self, status, body=b"", ctype="application/json", headers=None):
        if isinstance(body, (dict, list)):
            body = json.dumps(body).encode()
        elif isinstance(body, str):
            body = body.encode()
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)
        with lock, open(path_of("requests.log"), "a") as log:
            log.write("%d %s %s %d\n" % (self.server.server_port, self.command, self.path, status))

    def body_json(self):
        n = int(self.headers.get("Content-Length") or 0)
        try:
            return json.loads(self.rfile.read(n) or b"null")
        except ValueError:
            return None

    def serve_file(self, folder, rel):
        full = os.path.realpath(os.path.join(folder, rel))
        if not full.startswith(os.path.realpath(folder) + os.sep) or not os.path.isfile(full):
            return self.answer(404, "not found", "text/plain")
        with open(full, "rb") as f:
            data = f.read()
        if read("stall") and read("stall") == self.path.split("?", 1)[0]:
            return self.serve_stalled(data)
        self.answer(200, data, "application/octet-stream")

    def serve_stalled(self, data):
        """Half the file, then wait while STATE_DIR/stall exists (see the module doc)."""
        half = len(data) // 2
        try:
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data[:half])
            self.wfile.flush()
            with lock, open(path_of("requests.log"), "a") as log:
                log.write("%d %s %s stalled\n" % (self.server.server_port, self.command, self.path))
            open(path_of("stalled"), "w").close()
            for _ in range(600):
                if not os.path.exists(path_of("stall")):
                    break
                time.sleep(0.2)
            self.wfile.write(data[half:])
            self.wfile.flush()
        except OSError:
            # The launcher dropped the connection (the player cancelled).
            self.close_connection = True

    def serve_check(self, folder):
        if not os.path.isdir(folder):
            return self.answer(404, "not found", "text/plain")
        body, etag = manifest(folder)
        if self.headers.get("If-None-Match") == etag:
            return self.answer(304, b"", "text/plain", {"ETag": etag})
        self.answer(200, body, "text/plain; charset=utf-8", {"ETag": etag})

    # --- game port ---------------------------------------------------------
    def game(self):
        p = self.path.split("?", 1)[0]
        if p == "/v2/launcher":
            return self.answer(200, {"banners": [], "messages": [], "links": []})
        if p == "/v2/login" and self.command == "POST":
            req = self.body_json() or {}
            if req.get("username") != USER:
                return self.answer(400, "username-error", "text/plain")
            if req.get("password") != PASSWORD:
                return self.answer(400, "password-error", "text/plain")
            channel = read("channel", "stable")
            slot = channel[4:] if channel.startswith("pre:") else ""
            patch_server = ""
            if slot and slot in slots() and read("eligible", "no") == "yes":
                patch_server = "http://127.0.0.1:%d/prerelease/%s" % (GAME_PORT, slot)
            return self.answer(200, {
                "currentTs": 1, "expiryTs": 2, "entranceCount": 1, "notices": [],
                "user": {"tokenId": 1, "token": "game-token", "rights": 0},
                "characters": [{"id": 7, "name": "Tester", "isFemale": False, "weapon": 0,
                                "hr": 999, "gr": 1, "lastLogin": 0}],
                "mezFes": None, "patchServer": patch_server,
            })
        if p == "/check":
            return self.serve_check(path_of("stable"))
        if p.startswith("/prerelease/"):
            rest = p[len("/prerelease/"):]
            slot, _, sub = rest.partition("/")
            if slot not in slots():
                return self.answer(404, "not found", "text/plain")
            if sub == "check":
                return self.serve_check(path_of("slots", slot))
            return self.serve_file(path_of("slots", slot), sub)
        if p == "/v2/server/status":
            # What the live wrapper answers (2026-11-23): its old-config flags only know the festa.
            return self.answer(200, {"events": {"festaActive": True, "divaActive": False}})
        if p.startswith("/v2/") or p.startswith("/api/"):
            return self.answer(404, "not found", "text/plain")
        return self.serve_file(path_of("stable"), p.lstrip("/"))

    # --- account API -------------------------------------------------------
    def channel_answer(self):
        channel = read("channel", "stable")
        slot = channel[4:] if channel.startswith("pre:") else None
        eligible = read("eligible", "no") == "yes"
        return {
            "channel": channel, "slot": slot, "eligible": eligible, "confirmationRequired": not eligible,
            "slotsAvailable": True,
            "slots": [{"slot": s, "version": read("slots/%s.version" % s) or None,
                       "notes": ["Test note for " + s], "published": "2026-11-23T00:00:00Z"} for s in slots()],
            "characters": [], "currentPatch": read("stable.version") or None,
        }

    def api(self):
        p = self.path.split("?", 1)[0]
        if p == "/api/auth/signin" and self.command == "POST":
            req = self.body_json() or {}
            if req.get("username") == USER and req.get("password") == PASSWORD:
                return self.answer(200, {"accessToken": TOKEN, "tokenType": "Bearer", "id": 1,
                                         "username": USER, "roles": ["ROLE_USER"]})
            return self.answer(401, {"message": "Bad credentials"})
        if p == "/api/account/patch-channel":
            if self.headers.get("Authorization") != "Bearer " + TOKEN:
                return self.answer(401, {"message": "Unauthorized"})
            if self.command == "GET":
                return self.answer(200, self.channel_answer())
            if self.command == "PUT":
                req = self.body_json() or {}
                channel = req.get("channel")
                if channel == "stable":
                    write("channel", "stable")
                    write("eligible", "no")
                    return self.answer(200, {"channel": "stable", "slot": None, "eligible": False,
                                             "message": "Stable patch. Your launcher puts the stable patch back at its next login."})
                if not (isinstance(channel, str) and channel.startswith("pre:") and channel[4:] in slots()):
                    return self.answer(400, {"message": "That pre-release is not out."})
                if read("eligible", "no") != "yes":
                    if req.get("confirm") is not True:
                        return self.answer(409, {"message": CONFIRM, "confirmationRequired": True})
                    write("eligible", "yes")
                write("channel", channel)
                return self.answer(200, {"channel": channel, "slot": channel[4:], "eligible": True,
                                         "message": "Pre-release patch \"%s\" on. Your Mezeporta launcher downloads it at its next login." % channel[4:]})
        if p in ("/api/public/events/current", "/api/public/festa") and self.command == "GET":
            # STATE_DIR/events-current.json and festa.json; STATE_DIR/events-down makes both fail.
            name = "events-current.json" if p.endswith("current") else "festa.json"
            if os.path.exists(path_of("events-down")) or not os.path.isfile(path_of(name)):
                return self.answer(503, {"message": "unavailable"})
            with open(path_of(name), "rb") as f:
                return self.answer(200, f.read())
        return self.answer(404, {"message": "not found"})

    def release(self):
        return self.serve_file(path_of("release"), self.path.split("?", 1)[0].lstrip("/"))

    def route(self):
        port = self.server.server_port
        if port == GAME_PORT:
            return self.game()
        if port == API_PORT:
            return self.api()
        return self.release()

    do_GET = do_POST = do_PUT = do_HEAD = do_DELETE = route


def main():
    servers = [http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler)
               for port in (GAME_PORT, API_PORT, RELEASE_PORT)]
    for s in servers[1:]:
        threading.Thread(target=s.serve_forever, daemon=True).start()
    print("listening", flush=True)
    servers[0].serve_forever()


if __name__ == "__main__":
    main()
