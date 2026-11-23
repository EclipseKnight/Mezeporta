Mezeporta launcher {version} (Hunsterverse build)
=====================================================

This is the Mezeporta launcher v1.5.4 by LilButter
(https://github.com/LilButter/Mezeporta), with changes made by Hunsterverse:

- 1.5.4-hv1 (2026-11-05): patch downloads work on slow connections. A patch
  file fails only when no data arrives for 60 seconds (or after 30 minutes in
  total), instead of 10 seconds per file.
- 1.5.4-hv2 (2026-11-23):
  - Client patch choice: after logging in to Hunsterverse, the "Patch" button
    (bottom left of the character page) shows the installed patch and the one
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

Source code: Mezeporta-v{version}-source.zip on the same release page
(https://github.com/EclipseKnight/Mezeporta/releases), and the git history of
the fork's "hunsterverse" branch.

Licence: like Mezeporta itself, this build is free software under the GNU
General Public License, version 2; the full text follows below. It comes
with NO WARRANTY. Questions about this build go to Hunsterverse, not to
LilButter.

--------------------------------------------------------------------------
