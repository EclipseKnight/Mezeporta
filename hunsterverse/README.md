# Hunsterverse build of Mezeporta

This branch (`hunsterverse` of github.com/EclipseKnight/Mezeporta, a fork of
LilButter/Mezeporta v1.5.4, GPL-2.0) is the launcher our installers ship.
Everything we add is marked "Hunsterverse" in comments; our own files are in
this folder (including the Linux zip's `run-mezeporta.sh`, since hv6),
`src-tauri/src/hv_*.rs` and `src/components/HunsterversePanel.vue`.

| Version | Change |
|---|---|
| 1.5.4-hv1 (2026-11-05) | Patch downloads fail after 60 s without data instead of 10 s per file (`patcher.rs`, `server.rs`, `main.rs`) |
| 1.5.4-hv2 (2026-11-23) | Client patch choice, launcher self-update, safer patching (below) |
| 1.5.4-hv3 (2026-11-23) | The Patch button moved off the CAPCOM logo (classic: in the footer after "Online Players"; PS4 style: right of Settings), part of the launcher layout so it scales with the window; the header's "release ver." comes from `tauri.conf.json` (hv2 still said hv1); event badges from the website's event list (below); Proton on the Steam Deck (below) |
| 1.5.4-hv4 (2026-11-23) | Downloaded patch files are kept per server (patch cache, below): switching Stable <-> pre-release takes them from there instead of downloading again. Cancelling a patch stops it (nothing moves into the game folder); on Windows the return from the game to the launcher starts a fresh launcher process (below); `Mezeporta-Linux.zip` stores `mezeporta-bin` and the `*.sh` files as executable (below) |
| 1.5.4-hv5 (2026-11-23) | A self-update that needs administrator rights (launcher in Program Files, or files an administrator created) asks for them with the UAC prompt on Windows instead of failing with "access is denied"; clear messages when the prompt is declined, when it is still denied, on Linux, and when patching a protected game folder (below) |
| 1.5.4-hv6 (2026-11-24) | Includes hv5. Log files on disk (`Mezeporta/logs`, the current run and the 4 before), what the game start, login, patch and self-update do, the game's own output and exit code on Linux / Steam Deck, and a "Problem report" (Settings, Advanced: Save report, Copy report) for Discord; our own `run-mezeporta.sh` that logs too. `tauri-plugin-log` replaced by `hv_log.rs` ("Logs and problem reports", below). A game that closes within 60 s, crashes or does not start ("instant crash on launch"): its exit code in words (Windows NTSTATUS, Linux signals and Wine's codes; Windows too now, through a small watcher process), Wine's error lines, a message at the next launcher start offering "Save report", an optional debug launch on Linux (`WINEDEBUG=+seh,+loaddll`), and in the report the game files' sha256, hvmods' crash files and Windows Error Reporting entries (`hv_crash.rs`, below) |
| 1.5.4-hv7 (built 2026-11-24, not published) | Steam Deck: the game ended right after Start with exit code 5. `proton run` starts the game through Proton's `steam.exe`, which does not pass the launcher's stdin (the start data) on, so `meze-deps.exe` read nothing. Proton now runs `getcompatpath` (prefix set-up) then `runinprefix meze-deps.exe --stdin-b64`; plain Wine unchanged. A debug launch on Proton also gets Proton's own log where the Proton command passes the variables on (the installer's `proton-host.sh` does not yet). "The game's start data (hv7)" under "Proton on the Steam Deck", below |
| 1.5.4-hv8 (built 2026-11-24, Windows and Linux) | Includes hv7. The restart after "Update and restart" on Windows: "could not restart the launcher: The request is not supported. (os error 50)" and launchers that just closed. Up to hv7 the restart's spawn passed on the stale standard handles `FreeConsole` leaves behind; now those are cleared at start, the restart tries CreateProcess (no handles, no console), then ShellExecuteEx, then `explorer.exe`, watching each for an early end; the single-instance lock is released first; if nothing starts, a notice (not an error) says the update is installed and to open the launcher again. A new launcher whose predecessor (up to hv7) is still running after 20 s starts anyway instead of exiting as a "duplicate". The log says "started after an update from <version>" (Windows and Linux) and each restart attempt. "Restart after an update (hv8)", below |

## Client patch choice (hv2)

What a player sees: after logging in to Hunsterverse, a **Patch** button on
the character page ("Patch: Stable v1.30"; since hv3 in the classic footer
after "Online Players", in the PS4 style right of Settings:
`HunsterversePatchButton.vue`). It opens a
panel with

- **Installed**: what this launcher last put in the game folder, and whether
  it is up to date;
- **Server gives you**: the patch the game server answered at this login;
- **Choose**: Stable, and one button per published pre-release slot (with
  its version and "what you are testing" notes);
- while a pre-release is out (hv3): "Full pre-release notes on the website",
  which opens the Client patch tab of the website's Account page
  (`https://frontier.hunsterverse.net/account.html#client-patch`) in the
  browser.

One source of truth: the account's `users.patch_channel`, the setting the
website's Account page edits. The launcher signs in to the same REST API
with the player's game username and password (`POST /api/auth/signin`), reads
`GET /api/account/patch-channel` and saves with `PUT` (`stable` or
`pre:<slot>`). The API's rules apply unchanged: the first pre-release of an
account that is not staff or playtester answers 409 with an explanation;
the launcher shows it with "I understand" and sends the PUT again with
`confirm: true` (which gives the Playtester role, as on the site).

After a change the launcher logs in again. Erupe's login answer then carries
the new `patchServer` (`/prerelease/<slot>` for a pre-release, the wrapper's
own `/check` for stable; `server/api/prerelease.go` `patchServerFor`), and the
launcher's normal "Server Updates" dialog installs it: it restores the files
the previous patch replaced (`*.mezeb`), removes the files it added, then
downloads what differs (since hv4 from the patch cache when it has the
file, below). Pre-release and stable share the server key (`host:port`), so
it is one patch state per server.

The panel only appears for servers in `HUNSTERVERSE_HOSTS` (`hv_patch.rs`:
`129.213.54.221`) and talks only to `HUNSTERVERSE_API`
(`https://api.hvmhfz.eclipseforge.xyz`). The password stays in memory for the
session (also without "remember me"); the API token is not stored.

Versions shown come from the API (`currentPatch` for stable, each slot's
`version`), not from the game files. The launcher records what it installed
in `<game>/Mezeporta/Servers/<host_port>/hv-patch.json` (channel, `/check`
ETag, version once known).

## Safer patching (hv2, `patcher.rs`)

- The manifest of replaced and added files is saved after each file, before
  the file is moved. If moving fails half-way (a file in use on Windows, a
  full disk), the launcher at once puts back every file it already replaced
  or added and forgets the server's patch state, so the folder is as before
  the patch and the next login starts over. A crash in the middle leaves a
  manifest that the next login's restore uses.
- The restore walk (`restore_backup_files`) no longer follows symlinks: with
  a portable Wine prefix in the game folder it used to walk `dosdevices/z:`
  (the whole disk).
- Unchanged: downloads go to `<game>/tmp/` first and nothing moves until all
  of them arrived; a failed download leaves the folder with the original
  files and the next login tries again.

### Cancel (hv4 fix)

Up to hv4 as first built, Cancel on the patcher page only ended the download
loop early: the run then went on as if every file had arrived, moved what was
in `tmp/` (a half-written file included) into the game folder and, when the
cancelled file was the last one, recorded the patch as installed (the next
login offered nothing, with a broken file in place). Now a cancel ends the
run before anything moves (also when it comes while checking, or while the
files come from the patch cache), `tmp/` is deleted and no error is shown;
the next login offers the patch again. One run at a time: a new run waits
until a cancelled one has finished. After a cancel the game folder holds the
original game files (the run puts back what the previous patch replaced
before it downloads), with no patch recorded for the server; Cancel in the
"Server Updates" dialog itself changes nothing (the previous patch stays).

Windows: the game runs inside the launcher's process (`meze-butter` loads
`mhfo-hd.dll` there). When the game goes back to the launcher (exit code
102), upstream reopened the window in the same process, where the DLLs the
game loaded and files it kept open stay until the process ends; a patch at
the next login (pending because its dialog was cancelled before playing)
then could not replace them and failed at once, every login, until the
launcher was closed. Now that return starts a fresh launcher process (as the
self-update restarts, `hv_update::restart`), and only if that fails reopens
in the same process.

## Linux zip file modes (hv4, `pack_launcher_zip.py`)

Upstream's `Mezeporta-Linux.zip` is made on Windows and stores no Unix file
modes, so unzipping it over an install (the Play page's "Updating the
launcher" for launchers older than hv2) recreated `run-mezeporta.sh`, which
every start script runs, without its execute bit: the launcher did not open
(the Play page only said to `chmod +x mezeporta-bin`). Our Linux zip now
stores `mezeporta-bin` and every `*.sh` with mode 755; `build-release.sh`
stops if the zip lacks them. The installers set the modes themselves.

## Patch cache (hv4, `hv_patch_cache.rs`)

Every patch file the launcher downloads is kept in
`<game>/Mezeporta/PatchCache/<host_port>/files/` (the same server key as
`Mezeporta/Servers/<host_port>/`, but outside it: that folder is wiped
whenever the server's patch changes). Before downloading a changed file the
patcher looks there; a file with the manifest's hash goes into `tmp/` like a
download, so switching Stable -> pre-release -> Stable downloads only the
first time. Any server, each only from its own folder.

- Key: the hash of the `/check` line. The wrapper's `/check` and Erupe's
  `/prerelease/<slot>/check` give `<sha256 hex>\t/<path>`, so the key is the
  sha256 (content-addressed: a file both patches share is kept once). Servers
  with the native manifest give a CRC32; its key also holds the path
  (`crc32-<crc>-<16 hex of sha256(path)>`), so a hit is as safe as the
  patcher's own "this game file already matches" check.
- Safety: a download goes in only if its hash matches the manifest (one that
  does not is installed as before, just not kept); a cached file is hashed
  again after it was put into `tmp/`, and one that does not match is deleted
  and downloaded. Cache hits happen in the download stage, so the hv2 rules
  stay: nothing moves before every file is in `tmp/`, rollback as before.
  Files are hard-linked (no extra disk space; the installed game file and the
  cache share it), copied where links fail (FAT/exFAT, another volume).
  Caching never fails a patch: a write error (disk full, a locked file)
  stops caching for that run; a file that cannot be removed (Windows lock) is
  retried at the next cleanup.
- Cleanup after each completed patch (`index.json` lists the versions by
  `/check` ETag, with channel and last install): keep the 4 most recently
  installed versions and at most 2 GiB per server. The oldest go first; the
  newest version of each other channel goes last; the installed version is
  never dropped. Files no kept version lists are deleted.
- Clearing: Settings, Maintenance, "Reset patched files" also deletes the
  whole `Mezeporta/PatchCache` (the confirmation says so). The reset the
  launcher does before switching to another server keeps it.

## Launcher self-update (hv2, `hv_update.rs`)

What a player sees: about 2 s after the launcher opens, if our latest GitHub
release is newer, a bar at the top: "Launcher update 1.5.4-hv3 (you have
1.5.4-hv2)", the release notes, **Update and restart** and **Later**.

How it works:

1. `GET https://github.com/EclipseKnight/Mezeporta/releases/latest/download/latest.json`
   (GitHub sends the asset of the newest release that is not a draft or a
   pre-release; a GitHub pre-release therefore never reaches players).
2. Newer means a higher `major.minor.patch`, then a higher `-hvN`.
3. Download the binary for this platform (`windows-i686`: `Mezeporta.exe`,
   `linux-x86_64`: `mezeporta-bin`), check its minisign signature against the
   public key built into the launcher, and that it is an executable for this
   system. Anything else is refused and nothing changes.
4. Write it as `<exe>.new` beside the running file, then swap: Linux renames
   it over `mezeporta-bin` (atomic; the old one kept as `.old` by a hard
   link); Windows renames the running `Mezeporta.exe` to `.old` (allowed for
   a running exe) and the new one into its name, renaming back on failure.
5. Restart: Linux `exec`s the new binary in the same process (same PID: on
   the Steam Deck the launcher is the main process of its Flatpak sandbox,
   which would end with it); Windows starts the new exe and exits, and the new
   one waits for the old PID before its single-instance check (hv8: several
   ways to start it, "Restart after an update (hv8)" below). The new
   launcher deletes `.old` at start.

Only the launcher binary changes. The other files of the release zips
(audio, DLLs, fonts, `run-mezeporta.sh`) stay as installed: a release that
changes them must say "run the installer again" in its notes.

Override for tests or staging: `MEZEPORTA_UPDATE_URL=<url of a latest.json>`
(the signature check still applies).

### Restart after an update (hv8)

Reports (2026-11-24, Windows): a player on hv4 pressed "Update and restart"
for hv6 and got, in red, `could not restart the launcher: The request is not
supported. (os error 50)` (the update itself was installed); others saw the
launcher simply close.

Cause (from the code; not reproduced, there is no Windows machine here and
Wine's console works differently): our Windows launcher is a console
program (no `windows_subsystem`) that calls `FreeConsole` in GUI mode. On
Windows 8 and later `FreeConsole` closes the standard handles Windows opened
for the console (a start from Explorer) but leaves their values in the
process (github.com/rprichard/win32-console-docs, "FreeConsole (modern)").
The restart was a plain `Command::spawn`, which inherits stdin/stdout/stderr:
Rust duplicates whatever object those values point at by then (or nothing)
and CreateProcess passes them on unchecked (`STARTF_USESTDHANDLES`). Which
object that is depends on the machine and the moment, so one PC fails with
error 50, another starts a launcher with odd handles. The hv6 game watcher,
which spawns with null handles, has no such problem. hv4's "back to the
launcher" restart used the same spawn.

Now:

- At start (GUI mode) the launcher clears the standard handles that were
  console handles after `FreeConsole` (files and pipes of a redirected start
  stay), so no spawn and no log line to stdout uses a stale value.
- `restart` (Windows, `hv_update.rs`) releases the single-instance mutex
  (`main.rs` `restart_launcher`), then tries, each logged: CreateProcess with
  null handles and `DETACHED_PROCESS` (no console window); `ShellExecuteExW`
  "open"; `explorer.exe "<exe>"` (Explorer starts it without our environment,
  so that launcher does not wait for the old PID; the mutex is already free).
  A started launcher that ends within 1.5 s counts as failed (it would be
  waiting for the old process), and the next way is tried.
- Nothing started: the update bar says, without the error colour, "The update
  is installed, but the launcher could not reopen itself (...). Close the
  launcher and open it again", and hides "Update and restart". The mutex is
  taken again. Same message when the Linux `exec` fails.
- The new launcher logs why it started (`MEZEPORTA_RESTARTED_FROM`, set by
  hv8 and later): "started after an update from 1.5.4-hv8 (previous process
  N)", "started again after the game", or for a restart by hv4-hv7 "started by
  a restart of an older launcher". Then (Windows) the wait: "has ended", "had
  already ended" or "still running after 20 s; starting anyway". In that
  last case it no longer exits as a duplicate when the old one still holds
  the mutex (launchers up to hv7 keep it until they are gone; such an exit
  left nothing in the log, as it happened before the log file opened).

Checked for "it just closes": the Windows wait (20 s) then the mutex: fixed
as above. The `.old` cleanup only logs a failure. A WebView2 start failure
(e.g. the old launcher's WebView2 still holding the data folder) ends the
launcher too; its log then shows `failed to receive message from webview`
or `error while building tauri application` just before `app exit`. Linux:
the `exec` uses the path read at start (before the rename, so not
`/proc/self/exe`'s "(deleted)" name) and the same arguments, in the same
Flatpak sandbox; no problem found there.

Players updating from hv4-hv7 still run the old restart once (the old
launcher restarts into the new one), so the error or the closing can happen
one last time; hv8's notes (shown in the old launcher's update bar) say the
update is installed and to open the launcher again.

### Updates that need administrator rights (hv5)

Report (2026-11-23): on Windows the update failed with "cannot move the
running launcher aside (...): Access is denied"; "Run as administrator"
fixed it. Cause: the launcher was in a folder the player cannot change
(under Program Files, or its files were created by an elevated process).
Our Windows build's manifest has no `requestedExecutionLevel`, so as a
32-bit program it runs with UAC file virtualization: in Program Files,
writing `Mezeporta.exe.new` "works" (Windows redirects it to
`%LOCALAPPDATA%\VirtualStore`), but renaming the real `Mezeporta.exe` is
denied. (We keep it that way: an `asInvoker` manifest would end the
virtualization that lets such installs save `Mezeporta/config.json` at
all.)

Now (`hv_update::install`):

1. The launcher tests its folder (creates and deletes a file). Writable: the
   swap as before. A swap that fails with access denied (step 4 of the
   list above, or removing an earlier `.old`) goes on to step 2; any other
   failure is shown as before.
2. Not writable or denied, Windows: the verified file is saved in the
   player's temp folder (`%TEMP%\Mezeporta-update-<pid>-<n>\`) and the
   installed launcher is started elevated (`ShellExecuteExW`, verb `runas`,
   the UAC prompt, no window) as `Mezeporta.exe --hv-apply-update <file>
   <signature>`. That step (`run_apply_update_helper`, first thing in
   `main`) reads the file once, checks those bytes against the public key
   built into it and that they are a Windows program, writes them beside
   itself and swaps (the same `replace_executable`), then exits. It only
   replaces its own file and never moves the temp file, so a program of the
   player that changes the temp file in between gets a refusal, not an
   elevated swap. The elevated program is the installed launcher (in the
   protected folder), never the download.
3. The normal launcher then restarts into the new version as before, so
   neither the launcher nor the game runs elevated.

Messages (the old launcher keeps working in every case; nothing is half
replaced):

| Case | Message (shortened) |
|---|---|
| Prompt declined | "The update needs administrator rights because the launcher is in <folder>. Click Update and restart again and allow the Windows prompt, or move the game folder somewhere you own, e.g. C:\Games." |
| Elevated and still denied, or the file in use | "Windows refused to replace <exe> even with administrator rights. Close every other Mezeporta window and check that your antivirus is not blocking or scanning Mezeporta.exe." |
| Linux / Steam Deck, folder not writable | "The update cannot write to the launcher's folder <folder>. Make that folder writable for your user (or move the game folder into your home folder)." |

The new launcher cannot delete the `.old` an elevated update left (it is
not elevated); that is only logged, and the next elevated update removes
it first.

Launchers up to hv4 have the old behaviour: on a protected install they
need "Run as administrator" once to take this update.

Patching (`patcher.rs`, same folder, same cause): renaming a game file to
`*.mezeb` in a protected folder is denied too. It is not done elevated; a
denied file operation now shows `patcher-permission-error` ("Patcher cannot
change files in the game folder (access denied) ... Move the game folder
somewhere you own, e.g. C:\Games") instead of the generic file error. The
rollback of a half-done patch is unchanged.

Testing the UAC step needs Windows (steps in the hv5 report; summary: put
the hv5 build in `C:\Program Files\Hunsterverse` as an administrator, run
it as a normal user against a test `latest.json` of a higher version via
`MEZEPORTA_UPDATE_URL`).

### Why our own updater, not Tauri's

Tauri 1's updater replaces an AppImage on Linux and runs an MSI or NSIS
installer on Windows. Our players run a plain binary inside the game folder
(upstream's layout: the launcher finds the game next to itself), from zips
and our installers. An installer would move it to Program Files; an AppImage
would make it look for the game inside its own mount. So the updater swaps
the one file. It uses Tauri's signature format and keys (`tauri signer`), so
moving to Tauri's updater later needs no new key.

### Why no AppImage for Linux / Steam Deck

Weighed 2026-11-23: an AppImage bundles WebKitGTK but not glibc, so it runs
only where glibc is at least the build machine's (Ubuntu 24.04: 2.39; an older
build host would be needed for older SteamOS); Tauri AppImages are known for
EGL/Mesa trouble with the bundled WebKit; it needs FUSE 2 (`libfuse.so.2`) or
the newer static runtime; and the launcher would look for the game inside the
AppImage mount (upstream already has special cases for that). The Flatpak
runtime way (installer part 245: `org.gnome.Platform//51`, user install, no
password, survives SteamOS updates) is proven on mhfz-dev and does not depend
on the binary, so self-updates keep working on the Deck: the new
`mezeporta-bin` is written into the game folder, which the sandbox can write,
and exec'd in the same sandbox. Not chosen either: Cirnodeck's way
(`steamos-readonly disable` + `pacman -S webkit2gtk-4.1`) needs the sudo
password and is wiped by every SteamOS update.

## Signing key

- Private key: `~/.secrets/mezeporta-updater.key` on mhfz-dev (mode 600,
  made with `npx tauri signer generate` on 2026-11-23), its password in
  `~/.secrets/mezeporta-updater.key.password`. Never commit or print them.
  Losing the key means no more self-updates for launchers that have its
  public half (players would install the next version by hand or with the
  installer, which then carries a new key). Keep a backup with the other
  secrets backups.
- Public key: `PUBLIC_KEY` in `src-tauri/src/hv_update.rs` (also
  `~/.secrets/mezeporta-updater.key.pub`), id `AF8EAC5CE5FE4371`.
- To change keys: put the new public key in `hv_update.rs` and release that
  version signed with the **old** key; sign everything after with the new one.

## Event badges (Hunsterverse events)

The badges at the bottom of the character page (upstream's
`src/activeEvents.js`) come from the wrapper's `/v2/server/status`, which
builds them from Erupe's old config (Festa/Diva/Tournament overrides,
EarthStatus) and the `events` table. On our server that only ever gives the
Hunter Festa: our own events are not in those flags. For servers in
`HUNSTERVERSE_HOSTS` the launcher also reads the REST API's public
`/api/public/events/current` and `/api/public/festa` (the website's Events
page, `js/events.js`; `hv_events.rs`, command `hv_live_events`), and
`src/hvEvents.js` turns them into badges, with the website's rules:

| Badge | Shown when | Art |
|---|---|---|
| Hunter Festa | the festa answer has a phase (entry, souls, judging, rewards); the panel names it | upstream |
| Speedrun Cup | its row's cycle runs and the phase is open, tallying or results | upstream Hunting Tournament |
| VS Bracket | its cycle runs: sign-up, drawing, a round, results (not upcoming or "no bracket") | ours, `VsBracket.png` |
| Diva Defense | its cycle runs (personal, tactics, rewards, between) | upstream |
| Event campaigns | open campaigns (the panel lists them) | ours, `Campaigns.png` |
| MezFes, Sky Corridor (upstream Tenro art), Raviente, Hunting Road vote | always (no schedule) | upstream; Raviente and Road vote ours |

Ours are drawn by `make_event_badges.py` (Raviente: the website's monster
icon). If `events/current` does not answer, the badges are the wrapper's
flags as before; if only `festa` fails, the wrapper's flag decides the Hunter
Festa. Pallone Festival and Conquest still come from the wrapper's flags.
Other servers are unchanged (`hv_live_events` answers null for them).

## Proton on the Steam Deck (hv3, `hv_proton.rs`)

On a Steam Deck the player installer (`hvmhfz_ops/player-installer`) sets
the Wine prefix mode to `proton` and starts the launcher with
`MEZEPORTA_PROTON_CMD` (its `proton-host.sh`, which runs Steam's Proton
outside the Flatpak runtime) and `STEAM_COMPAT_DATA_PATH`. SteamOS has no
Wine. Up to hv2 the settings page reset `proton` to `portable` whenever Dev
Mode was off, so the game start ran plain `wine` and failed ("failed to run
wine reg: No such file or directory"). Now:

- `proton` is never reset, and offered in Settings, Advanced, Wine Prefix
  without Dev Mode (`SettingsList.vue`).
- At game start, a Wine mode (portable, system, custom) with no Wine found
  and an existing `MEZEPORTA_PROTON_CMD` starts through Proton anyway
  (`resolve_launch_wine_runtime`), so players whose setting was already
  reset work without touching it.
- In Proton mode: no `wineserver -k` (Proton has its own), the controller
  override check looks at `$STEAM_COMPAT_DATA_PATH/pfx` (not `~/.wine`), and
  a missing `STEAM_COMPAT_DATA_PATH` gives a clear error. The missing-tools
  check already wanted only the Proton command (no wineserver, winetricks);
  the portable prefix install stays Wine only.

### The game's start data (hv7)

The launcher hands `meze-deps.exe` its start data (the `MhfConfig` JSON
with the session token and password, base64) on stdin. Up to hv6 the game
started as `proton run meze-deps.exe --stdin-b64`, and on a Steam Deck it
ended at once with exit code 5 ("start data missing or not readable":
`serde_json` failed on an empty string). Cause, found from a Deck
player's debug report and reproduced on mhfz-dev (GE-Proton11-14, Wine
9.0, 2026-11-24):

- `proton run` starts Proton's `steam.exe` (`steam_helper/steam.c`), which
  creates the game with `CreateProcessW(NULL, cmdline, NULL, NULL, FALSE,
  ...)`: no handle inheritance and no `STARTF_USESTDHANDLES`. The game gets
  no stdin handle (a probe exe saw handle 0, `ReadFile` error 6); Rust reads
  that as an empty stdin. Its stderr is lost too, hence no error line.
  Plain Wine starts the game directly with fd 0 as its stdin, which is why
  Linux players with Wine never saw it. The earlier Deck test used a
  stand-in Proton script, so it never ran `steam.exe`.
- Ruled out: `flatpak-spawn --host` passes stdin on (the real Flatpak
  runtime on mhfz-dev, through the installer's `proton-host.sh`), and
  upstream's `meze-deps.exe` in the Linux zip (sha256 `f4894263...`) reads
  this launcher's `MhfConfig` (the struct is unchanged since v1.5.4).

Now (`hv_proton::GAME_SCRIPT`, through `/bin/sh` inside the exit-code
wrapper): `proton getcompatpath "$PWD" </dev/null` first, which makes
Proton create or upgrade its prefix (DXVK and so on) and then only
converts a path, then `exec proton runinprefix meze-deps.exe --stdin-b64`,
which runs Proton's own wine on the game directly, with the launcher's
pipe as its stdin. The start data never touches the disk, the installer's
`proton-host.sh` needs no change (it passes `"$@"`), and Proton's
environment (prefix, DLL overrides, fsync) is the same as with `run`.
What `runinprefix` skips: `steam.exe` (without `SteamGameId`, which
`proton-host.sh` does not pass, it only started and awaited the game),
Proton's game and Steam drive letters, and xalia (its controller
accessibility helper). A failing `getcompatpath` is written to `game.log`
(`== proton getcompatpath (prefix set-up) ended with exit code N`) and the
game starts anyway. Without `/bin/sh` only `proton runinprefix` runs.

Weighed and not taken: `cmd.exe /c "meze-deps.exe --stdin-b64 < file"`
through `proton run` (the token and password in a file whose read we
cannot see, so not deletable at a known time; `steam.exe` starts a console
program such as `cmd.exe` with a new console); our own `meze-deps.exe` reading a file or
the environment (a new Windows build to ship and keep, the old one stays
on players' disks; `proton-host.sh` passes no new variables).

First start after the update: `getcompatpath` sets up a prefix that is
missing or from another Proton version (46 s for a new one on mhfz-dev;
the first hv6 start on the Deck took 417 s, likely the same). Later starts
add about a second.

## Logs and problem reports (hv6, `hv_log.rs`)

Up to hv5 the launcher logged to stdout and to the webview only
(`tauri-plugin-log`; nothing in the frontend listened), so a player's
machine kept nothing. hv6 replaces the plugin with its own logger: every
line goes to stdout as before and to a file.

### Where the files are

| System | Folder |
|---|---|
| Linux, Steam Deck | `<game folder>/Mezeporta/logs/`, beside `mezeporta-bin` and `Mezeporta/config.json` (installer default `~/Games/MHFZ/Mezeporta/logs/`). On the Deck the launcher runs in the Flatpak runtime sandbox, which shares exactly that folder, so the files are on the real disk. If it cannot be written: `$XDG_STATE_HOME/Mezeporta/logs` (else `~/.local/state/Mezeporta/logs`), then `/tmp/Mezeporta-logs` |
| Windows | `<launcher folder>\Mezeporta\logs\` (the folder the launcher runs in, where `Mezeporta\config.json` is). A launcher under Program Files writes to `%LOCALAPPDATA%\Mezeporta\logs\` instead: there UAC file virtualization would hide the files in `VirtualStore` (hv5 notes) |

The first lines of each run name the folder (`log folder: ...`), and the
report shows it.

| File | What |
|---|---|
| `mezeporta.log` | this run; `mezeporta.1.log` .. `mezeporta.4.log` the runs before. A run that passes 1 MiB moves on to a new file, so the files stay under about 5 MiB together. The file opens after the single-instance check (a second launcher must not move the running one's file aside); lines from before are written then |
| `game.log`, `game.1.log` | Each game start (hv6 crash part: "Game that closes or crashes at once", below). Linux / Steam Deck: the game's stdout and stderr (Wine or Proton and `meze-deps.exe`), from the start command to `== ... game process ended, exit code N, after S s`. Upstream threw this output away. The launcher closes once the game runs, so `/bin/sh -c '"$@"; ...'` around the game command writes the exit code (same command, arguments and stdin otherwise; only when that command exists and is executable, so a missing Wine or Proton still fails at once with the launcher's error; without `/bin/sh` the game starts as before, without the exit code). Not capped within one game session (the launcher is closed meanwhile); each game start begins a new file. The game's environment is unchanged (Proton sets `WINEDEBUG=-all` itself unless `PROTON_LOG`), except in a debug launch. Windows: the game runs inside the launcher process (meze-butter), so the file has only the start line and the end line (below) |
| `run-mezeporta.log`, `run-mezeporta.1.log` | Linux: written by `run-mezeporta.sh` (below) |
| `problem-report-<UTC time>.txt` | the reports (the newest 3 are kept) |

### What is in them

- Each line: `2026-11-24 10:15:03.123 UTC INFO  [module] message`. Our
  modules from debug up, libraries from warn up. A panic is logged too.
- At start: launcher version and file, working and log folder; OS and
  version (Linux: `/etc/os-release`; in a Flatpak sandbox the host's
  `/run/host/os-release`, i.e. SteamOS and its build, plus the runtime),
  kernel, device (`Valve Jupiter`/`Galileo` = Steam Deck); Wine found on PATH
  and its `wine --version`; the Proton command (`MEZEPORTA_PROTON_CMD`,
  `PROTON_CMD`) with Proton's own `version` file, also through the
  installer's `proton-host.sh`; the Proton prefix (`$STEAM_COMPAT_DATA_PATH/pfx`);
  Windows: version and build from the registry, Wine's version when the
  Windows launcher runs under Wine/Proton (`wine_get_version`), whether the
  launcher's folder is writable. Environment: only a fixed list
  (`ENV_NAMES`: Wine and Proton commands, `WINEPREFIX`, `WINEESYNC`/`FSYNC`,
  `WINEDEBUG`, `WINEDLLOVERRIDES`, `SteamDeck`, `SteamAppId`, display and
  session variables, `GDK_BACKEND`, the WebKit switches, `FLATPAK_ID`) and
  every `PROTON_*`, `STEAM_COMPAT_*`, `DXVK_*`, `VKD3D_*`; `LD_PRELOAD` only
  as "(set)".
- Login: server, mode and game folder; the outcome (characters, patch
  server, whether a patch is offered) or the error. Patch: each run (server,
  URL, ETag from and to, the server patched before), cache use, cancel,
  installed, every error. Patch choice (channel set, API errors). Self-update:
  check (latest version or error), install, the administrator step,
  restart, errors.
- Game start: the exact command (`".../proton" "runinprefix" ".../meze-deps.exe"
  "--stdin-b64" (after "getcompatpath": Proton's prefix set-up)`; up to hv6 `"run"`), runtime, folder, what the launcher sets (`WINEPREFIX`,
  x11 variables, `WINEESYNC=0`/`WINEFSYNC=0`, `STEAM_COMPAT_DATA_PATH`), game
  version, character id, server; the process id; on Windows the meze-butter
  start and the exit code. At the next start: the last game start's result
  (`last game start (...): the game ended after 3 s, exit code ...`, a
  warning with Wine's error lines when it ended early or crashed).
- What the player is shown: every message of the launcher's message box
  (`logMessage` in `store.js`, `[ui]` lines).

### What is never in them

- Passwords, the session token, the account API token, keys: the game's
  start data (which holds the token and password) goes to the game's stdin
  only and is never logged; the username is not logged either.
- As a second line of defence every line and the report go through
  `hv_log::redact`: the value after any field name containing `password`,
  `passwd`, `secret`, `token`, `authorization`, `apikey`/`api_key`,
  `private_key`, `cookie`, `session` or `credential` (`=`, `:`, JSON, Debug's
  `Some("...")`), `Bearer`/`Basic` credentials, JWTs and base64 runs of 160+
  characters become `[redacted]`.
- The home folder: `$HOME` / `%USERPROFILE%` is shown as `~` (paths under
  it keep the part after the home folder, e.g. `~/Games/MHFZ`).
  `run-mezeporta.log` is written by the shell and may hold the full path on
  disk; the report redacts it when it reads the file.

### Problem report

Settings, Advanced (both looks; `SettingsList.vue`), "Problem report":

- **Save report** writes `problem-report-<time>.txt` into the log folder,
  opens that folder (Windows: Explorer with the file selected; Linux:
  `xdg-open`, which in the Deck's sandbox may not open anything) and shows
  the file's path.
- **Copy report** puts the same text on the clipboard; pasted into a Discord
  message it becomes a `message.txt` attachment.

The report (`hv_log::build_report`): system (as at start, plus the Wine
version), the launcher's settings (look, language, server and its ports,
server mode, game folder, game version, Wine prefix mode and path,
controller DLLs, hardware acceleration, DPI fix, Dev Mode, logged in and the
patch server, patch offered, the active server, the installed patch record), the
crash part (next section: last game start, game files, hvmods' crash files,
Windows Error Reporting), then the end of each log: `mezeporta.log` and `.1` (1500 lines each), `.2`
(500), `game.log` (400) and `.1` (200), `run-mezeporta.log` (300) and `.1`
(150). The run that failed is usually `mezeporta.1.log` (the player opens
the launcher again to make the report). Strings in English and Japanese
(`hv-report-*`).

### Game that closes or crashes at once (hv6, `hv_crash.rs`)

The owner's main question for hv6: why the game crashes at once on launch
(Steam Deck and others). The launcher now catches that case:

- **End line in `game.log`.** Linux: `/bin/sh` around the game command
  writes `== <time> UTC game process ended, exit code N, after S s` (the
  launcher has closed by then). Windows: the game runs inside the launcher
  process, so before it starts the launcher starts itself again as a small
  watcher (`Mezeporta.exe --hv-game-watch <pid> <start> <log folder>`, no
  window, handled first thing in `main`, no single-instance lock, no log
  rotation). It waits for the launcher process and writes the full exit code
  (`exit code 3221225477 (0xC0000005), after 1 s`), also when the game
  crashes the process or ends it with ExitProcess. When the game returns to
  the launcher the launcher writes `game returned to the launcher, exit code
  102, ...` first; when meze-butter cannot start it, `game did not start:
  <why>` (meze-butter now keeps Windows' reason when `mhfo-hd.dll` /
  `mhfo.dll` does not load, e.g. `The specified module could not be found.
  (HRESULT 0x8007007E)`: a DLL it needs is missing).
- **Exit codes in words** (`describe_code`): Windows NTSTATUS / exception
  codes (0xC0000005 access violation, 0xC0000135 DLL not found, 0xC000007B
  bad image, 0xC0000142 DLL initialization failed, 0xC0000409 stack buffer
  overrun, 0xC00000FD stack overflow, 0xC0000374 heap corruption, 0xE06D7363
  C++ exception, ...). Linux: 126/127 (not executable / not found), 128+n
  signals (SIGSEGV, SIGABRT, SIGKILL: out of memory, ...), meze-deps.exe's
  own codes 1-6, and since Wine passes on only the last byte of a Windows
  code, the codes that byte may stand for (`5 (maybe: ...; or 0xC0000005
  access violation ...)`). Wine's error lines from the output (`err:module:`,
  `err:seh:`, `Unhandled page fault ...`, `wine: could not load ...`,
  meze-deps' errors; up to 20) are picked out.
- **The rule** (`verdict`): ended within 60 s (`EARLY_EXIT_SECS`) = early;
  a crash code at any time (Windows `0xC...` except 0xC000013A Ctrl+C,
  breakpoint, C++ exception; Linux SIGILL, SIGTRAP, SIGABRT, SIGBUS, SIGFPE,
  SIGKILL, SIGSEGV) = crash; `game did not start` = not started. Exit code
  102 (back to the launcher) is never early. No end line (game still
  running, or killed together with its shell) = nothing to say.
- **Message at the next start** (`HunsterversePanel.vue`, both looks):
  "The game closed right after it started" / "The game crashed" / "The game
  did not start", the result line (exit code in words, the first Wine error
  line), and Save report, Copy report, on Linux "Debug the next game start",
  Close. Any button appends `== ... launcher: noted` to `game.log`, so it
  shows once. English and Japanese (`hv-crash-*`).
- **Debug launch** (Linux: Wine or Proton): from the message or Settings,
  Advanced, Problem report, "Debug the next game start" (or
  `MEZEPORTA_GAME_DEBUG=1` for every start). The next game start gets
  `WINEDEBUG=+timestamp,+pid,+tid,+seh,+loaddll` (every exception and DLL
  load, with times and threads) into `game.log`, and its start line says
  `(debug launch: ...)`. Proton keeps a `WINEDEBUG` that is set; the
  installer's `proton-host.sh` forwards it. Plain starts stay as they
  were. The setting lasts for one start and is not saved.
- **Proton's own log** (hv7, debug launch only): `PROTON_LOG=1`,
  `PROTON_LOG_DIR=<log folder>` and `SteamGameId=hunsterverse`, so Proton
  writes `steam-hunsterverse.log` beside the other logs (its header: Proton
  version, options, kernel; then Wine's output, which then is not in
  `game.log`). Proton writes that file only with `SteamGameId` set, and with
  `PROTON_LOG` but no `SteamGameId` it sets `WINEDEBUG=-all` and the debug
  output is lost (checked with GE-Proton11-14). So the launcher sets them
  only when the Proton command passes them on: a command that uses
  `flatpak-spawn` (the installer's `proton-host.sh`) must name all three,
  anything else is taken to pass its environment on
  (`hv_proton::proton_log_passes`). Today's `proton-host.sh` names only
  `PROTON_LOG`, so on a Deck the debug launch stays as in hv6 (`WINEDEBUG`
  into `game.log`, which works) and the start line says "no Proton log".
  Each Proton start deletes an older `steam-hunsterverse.log`. The report
  has its first 40 and last 400 lines.
- **In the report**: "Last game start (game.log)" (start time, result,
  verdict, Wine's error lines); "Game files in <folder>": `mhf.exe`,
  `mhfo-hd.dll`, `mhfo.dll`, `hvmods.dll`, `mhf.ini`,
  `Mezeporta/bin/meze-deps.exe` with size, time and sha256, or MISSING (the
  DLLs carry no version resource we could rely on; the hash names the patch
  version against the patch server's `/check`); hvmods' own files in the
  game folder (hvmhfz_re `mods/hvmods`: its crash catcher writes
  `hvcrash-<UTC time>.txt` and `hvdiag.txt` before uploading the crash at the
  next game start from `hvsend-crash.txt`): the newest 5 crash file names,
  the newest one (250 lines), `hvsend-crash.txt`, `hvmods.log` (150),
  `hvdiag.txt` (200); Windows: Windows Error Reporting entries
  (`%LOCALAPPDATA%` and `%ProgramData%\Microsoft\Windows\WER\ReportArchive`
  and `ReportQueue`, `AppCrash_Mezeporta.exe_*` and `AppCrash_mhf.exe_*`: the
  `Report.wer` fields, fault module, exception code in words, offset) and
  crash dumps in `%LOCALAPPDATA%\CrashDumps` (name and size only, not
  attached), newest 5.

### `run-mezeporta.sh` (Linux)

Every Linux start script runs it (the installer's `run-hunsterverse.sh`
too). Ours (in this folder; `pack_launcher_zip.py` puts it into
`Mezeporta-Linux.zip` instead of upstream's, `build-release.sh` checks)
writes `Mezeporta/logs/run-mezeporta.log` (the start before as `.1`): the
time, kernel, OS (host and sandbox), Flatpak runtime, display and Proton
variables, then everything the launcher prints. So a launcher that never
opens (`libwebkit2gtk-4.1.so.0: cannot open shared object file`, "cannot
open display", a crash) still leaves a trace. Not in a terminal it `exec`s
the launcher with its output appended (same process as before: on the Deck
the launcher stays the sandbox's main process) and sets
`MEZEPORTA_LOG_TO_STDOUT=0`, so the launcher prints only the lines from
before its own file is open; in a terminal it shows the output too (`tee`)
and adds the exit code. If the folder cannot be written it starts the
launcher as before.

The self-update replaces only the binary: players get this script from the
zips, i.e. the installer (run it again) or a by-hand unzip. The launcher's
own log works without it.

Not covered: the installer's `run-hunsterverse.sh` itself (Flatpak runtime
missing, `flatpak run` failing) runs before this script; that is the
installer's part (`hvmhfz_ops/player-installer`).

### Sending it to us later

The REST API has `POST /api/account/diagnostics` (website sign-in; the
launcher already signs in to the same API for the patch choice). It takes
the game's `hvdiag.txt` / `hvcrash-*.txt` and `*.log` files, but explains a
`.log` as an `hvmods.log`, and keeps 256 KB. A launcher upload ("Send report
to Hunsterverse") needs an API change first: a launcher report kind (its own
explainer or none, area `launcher`). Not done in hv6.

## Making a release

1. Bump the version in `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`
   (`1.5.4-hvN`), write `hunsterverse/release-notes/<version>.txt` (players see
   the first lines in the update bar).
2. `flock /srv/devbot/.heavy-build.lock hunsterverse/build-release.sh` (about
   20-30 minutes on mhfz-dev from scratch). Output in
   `/root/work/mezeporta-release/out/`.
3. `hunsterverse/test-e2e.sh` with this build and one with a higher version
   (see the script's header), `cargo test --target x86_64-unknown-linux-gnu hv`
   in `src-tauri`, and `bash hunsterverse/tests/run-mezeporta.test.sh`.
4. Commit, tag `v<version>`, push (owner's go: public), then a GitHub release
   on the tag with **every** file of `out/`. Upload `latest.json` last: from
   then on launchers see the update. Do not mark it pre-release (players would
   not get it), unless that is the point.
5. Update the installer pins in `hvmhfz_ops/player-installer` (URL, size and
   sha256 of the two zips).

Rollback: delete `latest.json` from the release (or the release), so no new
launcher updates; launchers already updated stay on it until a newer release.

## Tests

- `cd src-tauri && cargo test --release --target x86_64-unknown-linux-gnu hv`:
  version order, manifest, signatures (good, changed, wrong key), file swap
  and its failure, the channel from a patchServer URL, the installed record,
  and the patcher: a move that fails half-way is rolled back, a move then
  restore round trip, the restore walk not following symlinks, and when the
  game starts through Proton (`hv_proton`), and the patch cache (hv4: hit,
  miss, a corrupted cache file, a download that does not match its hash,
  per-server folders, cleanup order and the size cap, `hv_patch_cache`). (The
  `--target` matters: `.cargo/config.toml` defaults to Windows i686.)
  Since hv5 also the update decision (writable folder: in place; access
  denied or not writable: the elevated step; declined, still denied, Linux
  messages), the elevated step's own check of the bytes (changed file,
  wrong key, not an executable, then a good swap) and the patcher's
  access-denied message.
- `hunsterverse/test-e2e.sh NEW_BIN NEXT_BIN` (env `NEXT_VERSION`, `NEW_VERSION`,
  `SCENARIOS`, `SHOTS`: see the script's header): the real Linux launcher under
  Xvfb, driven through tauri-driver, against local stand-ins of the wrapper,
  Erupe's pre-release routes, the account API and the release (no real
  server, account or game data): first login installs stable; the panel
  shows installed/served and the pre-release notes link (clicked through a
  stand-in `xdg-open` that records the URL; screenshot with `SHOTS`); pre-release with the confirmation, then back to
  Stable, then the pre-release again, with the game folder checked after
  each; the last two switches download no file (hv4 patch cache, counted
  in the stand-in's request log); the Patch button inside
  the launcher layout covering nothing at the default and a small window size
  (screenshots with `SHOTS`); the event badges from a stand-in events API
  (classic and PS4 style, screenshots), and the wrapper's flags while that
  API is down; Steam Deck: with no Wine and a stand-in Proton, Start runs
  `proton getcompatpath <game folder>` with an empty stdin, then `proton
  runinprefix meze-deps.exe --stdin-b64` with the game settings on stdin
  (hv7; setting `portable` or `proton`; a debug launch also gets
  `PROTON_LOG=1`, `PROTON_LOG_DIR` and `SteamGameId`),
  and `proton` stays set with Dev Mode off; a tampered update refused, the signed one installed and
  restarted in place; a cancelled patch (hv4 fix, the owner's steps): the
  pre-release chosen, its update dialog cancelled and the game started on the
  stable files; at the next launcher start the update cancelled while its
  last file downloads (the stand-in stalls it) leaves no file of it in the
  game folder, and the next login installs it completely. (WebDriver cannot
  follow a window the launcher reopens in the same process, so the game's
  "back to the launcher" is not driven; there the launcher closes and opens
  again.)
- `node --test hunsterverse/tests/*.test.mjs`: the event badge rules
  (`src/hvEvents.js`).
- hv6, in the `hv` tests above (`hv_log`): redaction (passwords, tokens,
  bearer and basic credentials, Debug `Some("...")`, cookies, private keys,
  JWTs, base64 blobs, the home folder; words without a value stay), the line
  format and UTC time, rotation (6 runs keep 5 files; a run past the size
  limit moves on, total bounded), the report (sections, logs, no password,
  newest 3 kept), tails, the Program Files check.
- hv6 crash part, in the same `hv` tests (`hv_crash`): the Windows codes
  in words, the Linux statuses (signals, 126/127, Wine's last byte), the
  early-exit rule (60 s, crash codes at any time, 102, Ctrl+C, no end
  line), `game.log` parsing (Linux with Wine error lines and a debug launch,
  Windows watcher line, returned then watcher, did not start, still
  running, noted), the real `/bin/sh` wrapper (exit 5 after 0 s: early;
  SIGSEGV: 139, crash), game files with sha256 and MISSING, hvmods' crash
  files, a UTF-16 `Report.wer` and a crash dump.
- hv7, in the same `hv` tests (`hv_proton`, `hv_log`): the Proton game
  script run by `/bin/sh` with a stand-in Proton (a path with a space):
  `getcompatpath <folder>` with an empty stdin, then `runinprefix
  meze-deps.exe --stdin-b64` with all of stdin and its exit code; a failed
  `getcompatpath` is said and the game still starts; without a shell only
  `runinprefix`, never `run`; when the Proton log variables get through
  (Proton itself, today's `proton-host.sh`, one that names them); the
  report's Proton log (first 40 and last 400 lines).
- hv7, by hand on mhfz-dev with real GE-Proton11-14 and upstream's
  `meze-deps.exe` (`f4894263...`), the launcher's own payload (from the e2e
  run; its stand-in token padded to 16 characters like a real one), the two scripts taken from the source: hv6's `proton run` ends with
  exit code 5 after 1-2 s (directly and through the installer's
  `proton-host.sh` inside `org.gnome.Platform//51` with `flatpak-spawn
  --host`); hv7's start creates a new prefix and meze-deps reads its start
  data in both set-ups, then stops at the next steps of a test folder with
  no game: `unable to find path to game` (no `mhf.ini`), with an empty
  `mhf.ini` `unable to load or free mhfo dll` (a stand-in DLL).
- hv8, in the same `hv` tests (`hv_update`): why the new launcher started
  (update from hv8, back from the game, an older launcher, a normal start),
  the restart trying each way until one starts and its failure message ("the
  update is installed", not the old error), Windows command-line quoting for
  ShellExecuteEx, and the Linux `exec` failure message. `test-e2e.sh`
  scenario `update` with `NEW_VERSION` hv8 or later: the restarted launcher's
  `mezeporta.log` says "started after an update from NEW_VERSION". Under
  Wine 9 (by hand): the Windows exe started with the restart variables logs
  the note and "has ended" / "had already ended" / "still running after 20
  s; starting anyway" (a Windows process of 9 s, a missing PID, one of 40 s).
  The Windows restart itself needs WebView2, which Wine lacks: not run here.
- `bash hunsterverse/tests/run-mezeporta.test.sh`: `run-mezeporta.sh` with a
  stand-in launcher: not a terminal (output and errors in the log, exit code
  kept), rotation, a terminal (`script`), no writable log folder.
- `test-e2e.sh` scenario `report` (hv6), after `proton` and `cancel` started
  the game: `game.log` has the stand-in Proton's start and exit code 0;
  Settings, Advanced, Save report writes the report (system, settings, the
  earlier runs' game start, `game.log`), opens the log folder (stand-in
  `xdg-open`) and shows the path; at most 5 launcher log files; the login's
  password in no file of the folder. The stand-in game ends at once, so the
  scenario also checks the early-exit message at the next start (title,
  exit code, its Save report, Close noted in `game.log`), the report's
  "Last game start" and "Game files", and that "Debug the next game start"
  reaches the stand-in Proton as `WINEDEBUG` (hv7: with `PROTON_LOG=1`,
  `PROTON_LOG_DIR` = the log folder and `SteamGameId=hunsterverse`, none of
  them on the plain starts before).
