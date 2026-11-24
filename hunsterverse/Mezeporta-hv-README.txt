Mezeporta launcher {version} (Hunsterverse build)
=====================================================

This is the Mezeporta launcher v1.5.4 by LilButter
(https://github.com/LilButter/Mezeporta), with changes made by Hunsterverse:

- 1.5.4-hv1 (2026-11-05): patch downloads work on slow connections. A patch
  file fails only when no data arrives for 60 seconds (or after 30 minutes in
  total), instead of 10 seconds per file.
- 1.5.4-hv2 (2026-11-23):
  - Client patch choice: after logging in to Hunsterverse, the "Patch" button
    (on the character page) shows the installed patch and the one
    the server gives your account, and lets you choose Stable or a
    pre-release. It is the same setting as the website's Account page; the
    launcher logs in again and installs the chosen patch.
  - Self-update: at start the launcher checks
    https://github.com/EclipseKnight/Mezeporta/releases for a newer version
    and offers "Update and restart". Only the launcher file itself is
    replaced, and only after its signature checks out.
  - A patch that fails while putting files in place puts the game files back
    as they were before (no half-patched game folder).
  - Restoring the original files no longer follows links out of the game
    folder (a portable Wine prefix inside it made it walk the whole disk).
- 1.5.4-hv3 (2026-11-23): the "Patch" button moved next to "Online Players"
  (classic look; right of Settings in the PS4 look), so it no longer covers
  the CAPCOM logo. The header shows the launcher's real version again.
  Event badges come from the Hunsterverse event list (Speedrun Cup, VS
  Bracket, Raviente, campaigns and more). Steam Deck: the "Proton" Wine
  prefix setting is kept (it was reset to "portable", so starting the game
  failed with "failed to run wine reg") and offered without Dev Mode; with
  no Wine installed the launcher uses the installer's Proton by itself.
- 1.5.4-hv4 (2026-11-23): patch files the launcher downloaded are kept per
  server (Mezeporta/PatchCache), so switching between Stable and a
  pre-release and back takes them from there instead of downloading them
  again (each file's hash is checked first). Settings, "Reset patched files"
  also deletes them.
- 1.5.4-hv5 (2026-11-23): Windows: when the launcher is in a protected
  folder (such as Program Files), its updates ask for administrator rights
  with the Windows prompt instead of failing with "access is denied".
  Patching a protected game folder says so and how to fix it (move the game
  folder somewhere you own, e.g. C:\Games).
- 1.5.4-hv6: log files and a problem report. The launcher keeps its log
  in Mezeporta/logs beside it (this run and the four before; on Linux also
  the game's own output and what run-mezeporta.sh saw). Settings, Advanced,
  "Problem report": "Save report" writes one text file with your system,
  the launcher's settings and its recent logs and opens its folder; "Copy
  report" puts the same text on the clipboard. Attach it to your help post
  on Discord. Passwords, session tokens and keys are never in it.
  If the game closes within a minute, crashes or does not start, the
  launcher says so when you open it again (with the reason it can tell)
  and offers "Save report"; on Linux "Debug the next game start" adds
  Wine's debug output to the next start's log.
- 1.5.4-hv7: Steam Deck: the game no longer closes right after Start
  (exit code 5). Proton did not hand the launcher's start data on to the
  game; the launcher now starts the game through Proton in a way that does.

Source code: Mezeporta-v{version}-source.zip on the same release page
(https://github.com/EclipseKnight/Mezeporta/releases), and the git history of
the fork's "hunsterverse" branch.

Licence: like Mezeporta itself, this build is free software under the GNU
General Public License, version 2; the full text follows below. It comes
with NO WARRANTY. Questions about this build go to Hunsterverse, not to
LilButter.

--------------------------------------------------------------------------
