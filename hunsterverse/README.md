# Hunsterverse build of Mezeporta

This branch (`hunsterverse` of github.com/EclipseKnight/Mezeporta, a fork of
LilButter/Mezeporta v1.5.4, GPL-2.0) is the launcher our installers ship.
Everything we add is marked "Hunsterverse" in comments; our own files are in
this folder, `src-tauri/src/hv_*.rs` and `src/components/HunsterversePanel.vue`.

| Version | Change |
|---|---|
| 1.5.4-hv1 (2026-11-05) | Patch downloads fail after 60 s without data instead of 10 s per file (`patcher.rs`, `server.rs`, `main.rs`) |
| 1.5.4-hv2 (2026-11-23) | Client patch choice, launcher self-update, safer patching (below) |
| 1.5.4-hv3 (2026-11-23) | The Patch button moved off the CAPCOM logo (classic: in the footer after "Online Players"; PS4 style: right of Settings), part of the launcher layout so it scales with the window; the header's "release ver." comes from `tauri.conf.json` (hv2 still said hv1); event badges from the website's event list (below); Proton on the Steam Deck (below) |
| 1.5.4-hv4 (2026-11-23) | Downloaded patch files are kept per server (patch cache, below): switching Stable <-> pre-release takes them from there instead of downloading again |

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
   one waits for the old PID before its single-instance check. The new
   launcher deletes `.old` at start.

Only the launcher binary changes. The other files of the release zips
(audio, DLLs, fonts, `run-mezeporta.sh`) stay as installed: a release that
changes them must say "run the installer again" in its notes.

Override for tests or staging: `MEZEPORTA_UPDATE_URL=<url of a latest.json>`
(the signature check still applies).

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

## Making a release

1. Bump the version in `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`
   (`1.5.4-hvN`), write `hunsterverse/release-notes/<version>.txt` (players see
   the first lines in the update bar).
2. `flock /srv/devbot/.heavy-build.lock hunsterverse/build-release.sh` (about
   20-30 minutes on mhfz-dev from scratch). Output in
   `/root/work/mezeporta-release/out/`.
3. `hunsterverse/test-e2e.sh` with this build and one with a higher version
   (see the script's header), and `cargo test --target x86_64-unknown-linux-gnu hv`
   in `src-tauri`.
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
  `proton run meze-deps.exe --stdin-b64` (setting `portable` or `proton`),
  and `proton` stays set with Dev Mode off; a tampered update refused, the signed one installed and
  restarted in place.
- `node --test hunsterverse/tests/*.test.mjs`: the event badge rules
  (`src/hvEvents.js`).
