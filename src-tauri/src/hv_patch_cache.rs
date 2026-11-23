//! Hunsterverse (1.5.4-hv4): keep downloaded patch files per server, so that
//! switching between Stable and a pre-release (or back) does not download the
//! same files again.
//!
//! Layout: `<game>/Mezeporta/PatchCache/<host_port>/` (the same server key as
//! `Mezeporta/Servers/<host_port>/` and the manifests; kept out of that folder
//! because `clear_server_patch_state` wipes it whenever the patch changes):
//!
//! - `files/<key>`: one file per content. The key is the file hash of the
//!   patch server's `/check` line: a sha256 (the wrapper's and Erupe's
//!   `/prerelease/<slot>/check` answers: `<sha256 hex>\t/<path>`) is the key
//!   as is; a CRC32 (servers with the native manifest) is not unique enough
//!   across files, so its key also carries the target path:
//!   `crc32-<crc>-<16 hex of sha256(path)>`, which makes a hit exactly as
//!   safe as the patcher's own "this game file already matches" check.
//! - `index.json`: the patch versions (by `/check` ETag) whose files are kept,
//!   with their channel and when they were last installed.
//!
//! Rules: a file goes in only after its hash matched the manifest, and comes
//! out only into the patcher's `tmp/` folder, where it is hashed again before
//! it counts (a mismatch drops the cache file and downloads it). Files are
//! hard-linked where the file system allows (no extra disk space; a copy
//! otherwise). Caching never fails a patch: any error (disk full, a locked
//! file) just means that file is downloaded or not kept.
//!
//! Cleanup after each completed patch: keep the [`KEEP_VERSIONS`] most
//! recently installed versions and at most [`MAX_BYTES`]; the installed
//! version and the newest version of every other channel go last, and the
//! installed version is never dropped.

use crate::patcher::checksum_matches;
use log::warn;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

pub const CACHE_DIR: &str = "Mezeporta/PatchCache";
const FILES_DIR: &str = "files";
const INDEX_FILE: &str = "index.json";
const PART_SUFFIX: &str = ".part";
/// Patch versions kept per server (installed, the other channel, and the
/// previous one of each).
pub const KEEP_VERSIONS: usize = 4;
/// Size cap per server.
pub const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

fn sanitize_server_key(server: &str) -> String {
    server
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

pub fn server_dir(root: &Path, server: &str) -> PathBuf {
    root.join(CACHE_DIR).join(sanitize_server_key(server))
}

/// The cache key for a manifest line, or None for a hash we do not know.
pub fn cache_key(hash: &str, target_path: &str) -> Option<String> {
    let hash = hash.trim().to_ascii_lowercase();
    if !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    match hash.len() {
        64 => Some(hash),
        8 => {
            let path = format!("{:x}", sha2::Sha256::digest(target_path.as_bytes()));
            Some(format!("crc32-{}-{}", hash, &path[..16]))
        }
        _ => None,
    }
}

fn remove_force(path: &Path) -> io::Result<()> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        let mut perms = meta.permissions();
        if perms.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = fs::set_permissions(path, perms);
        }
    }
    fs::remove_file(path)
}

/// Hard link `src` as `dst`, or copy it when linking is not possible (another
/// volume, FAT/exFAT). A failed copy leaves no `dst`.
fn link_or_copy(src: &Path, dst: &Path) -> io::Result<()> {
    if fs::symlink_metadata(dst).is_ok() {
        remove_force(dst)?;
    }
    if fs::hard_link(src, dst).is_ok() {
        return Ok(());
    }
    if let Err(e) = fs::copy(src, dst) {
        let _ = fs::remove_file(dst);
        return Err(e);
    }
    Ok(())
}

/// One server's cache during a patch run.
pub struct PatchCache {
    files: PathBuf,
    /// Set false after a failed write (disk full...): no more tries this run.
    writable: bool,
}

impl PatchCache {
    pub fn open(root: &Path, server: &str) -> Self {
        PatchCache {
            files: server_dir(root, server).join(FILES_DIR),
            writable: true,
        }
    }

    /// Put the cached file for (hash, target_path) at `dest` (in the patcher's
    /// tmp folder). True only if `dest` now holds a file with that hash; a
    /// cached file with another hash is dropped.
    pub fn take(&self, hash: &str, target_path: &str, dest: &Path) -> bool {
        let Some(key) = cache_key(hash, target_path) else {
            return false;
        };
        let cached = self.files.join(&key);
        if !cached.is_file() {
            return false;
        }
        if let Err(e) = link_or_copy(&cached, dest) {
            warn!("patch cache: could not use {}: {}", cached.display(), e);
            return false;
        }
        if checksum_matches(dest, hash) {
            return true;
        }
        warn!("patch cache: {} does not match its hash, downloading it", key);
        let _ = remove_force(dest);
        let _ = remove_force(&cached);
        false
    }

    /// Keep a downloaded file, if its hash matches the manifest. Never fails
    /// the patch.
    pub fn store(&mut self, hash: &str, target_path: &str, downloaded: &Path) {
        if !self.writable {
            return;
        }
        let Some(key) = cache_key(hash, target_path) else {
            return;
        };
        if !checksum_matches(downloaded, hash) {
            warn!("patch cache: {} does not match the manifest hash, not kept", target_path);
            return;
        }
        let cached = self.files.join(&key);
        if cached.is_file() && checksum_matches(&cached, hash) {
            return;
        }
        let part = self.files.join(format!("{key}{PART_SUFFIX}"));
        let result = fs::create_dir_all(&self.files)
            .and_then(|_| link_or_copy(downloaded, &part))
            .and_then(|_| fs::rename(&part, &cached));
        if let Err(e) = result {
            warn!("patch cache: could not keep {}: {} (no more caching this run)", target_path, e);
            let _ = fs::remove_file(&part);
            self.writable = false;
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CachedVersion {
    /// The /check ETag.
    pub etag: String,
    /// "stable" or "pre:<slot>".
    pub channel: String,
    /// Higher = installed more recently.
    pub last_used: u64,
    /// Cache keys of every file of this version.
    pub files: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Index {
    #[serde(default)]
    pub seq: u64,
    #[serde(default)]
    pub versions: Vec<CachedVersion>,
}

fn load_index(dir: &Path) -> Index {
    fs::read_to_string(dir.join(INDEX_FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_index(dir: &Path, index: &Index) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{INDEX_FILE}.tmp"));
    fs::write(&tmp, serde_json::to_vec_pretty(index)?)?;
    fs::rename(tmp, dir.join(INDEX_FILE))
}

/// After a completed patch: note the installed version (its files' keys from
/// the manifest lines `(hash, target_path)`) and clean up.
pub fn remember_installed(
    root: &Path,
    server: &str,
    etag: &str,
    channel: &str,
    entries: &[(String, String)],
) {
    let dir = server_dir(root, server);
    let mut index = load_index(&dir);
    index.seq += 1;
    let files: Vec<String> = entries
        .iter()
        .filter_map(|(hash, path)| cache_key(hash, path))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let etag = etag.trim().to_string();
    index.versions.retain(|v| v.etag != etag);
    index.versions.push(CachedVersion {
        etag: etag.clone(),
        channel: channel.to_string(),
        last_used: index.seq,
        files,
    });
    prune(&dir, &mut index, &etag, KEEP_VERSIONS, MAX_BYTES);
    if let Err(e) = save_index(&dir, &index) {
        warn!("patch cache: could not save {}: {}", dir.join(INDEX_FILE).display(), e);
    }
}

/// Drop old versions (see the module doc), then every cache file no kept
/// version lists (and leftovers of interrupted writes).
fn prune(dir: &Path, index: &mut Index, installed: &str, keep: usize, max_bytes: u64) {
    let files_dir = dir.join(FILES_DIR);
    let mut sizes: BTreeMap<String, u64> = BTreeMap::new();
    if let Ok(entries) = fs::read_dir(&files_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            sizes.insert(name, size);
        }
    }

    // Newest first.
    index.versions.sort_by(|a, b| b.last_used.cmp(&a.last_used));
    let installed_channel = index
        .versions
        .iter()
        .find(|v| v.etag == installed)
        .map(|v| v.channel.clone())
        .unwrap_or_default();
    // Rank: 0 = installed (never dropped), 1 = newest of another channel
    // (dropped last), 2 = the rest.
    let mut seen_channels = BTreeSet::new();
    seen_channels.insert(installed_channel);
    let rank: Vec<u8> = index
        .versions
        .iter()
        .map(|v| {
            if v.etag == installed {
                0
            } else if seen_channels.insert(v.channel.clone()) {
                1
            } else {
                2
            }
        })
        .collect();
    let mut kept: Vec<bool> = vec![true; index.versions.len()];

    // Version count: drop the oldest of rank 2 first, then of rank 1.
    for r in [2u8, 1] {
        for i in (0..index.versions.len()).rev() {
            if kept.iter().filter(|k| **k).count() <= keep {
                break;
            }
            if rank[i] == r && kept[i] {
                kept[i] = false;
            }
        }
    }
    // Size: the same order.
    let total = |kept: &[bool], versions: &[CachedVersion]| -> u64 {
        let keys: BTreeSet<&String> = versions
            .iter()
            .zip(kept)
            .filter(|(_, k)| **k)
            .flat_map(|(v, _)| v.files.iter())
            .collect();
        keys.iter().map(|k| sizes.get(*k).copied().unwrap_or(0)).sum()
    };
    for r in [2u8, 1] {
        for i in (0..index.versions.len()).rev() {
            if total(&kept, &index.versions) <= max_bytes {
                break;
            }
            if rank[i] == r && kept[i] {
                kept[i] = false;
            }
        }
    }

    let mut it = kept.iter();
    index.versions.retain(|_| *it.next().unwrap_or(&true));
    let wanted: BTreeSet<&String> = index.versions.iter().flat_map(|v| v.files.iter()).collect();
    for name in sizes.keys() {
        if !wanted.contains(name) {
            let path = files_dir.join(name);
            if let Err(e) = remove_force(&path) {
                warn!("patch cache: could not remove {}: {}", path.display(), e);
            }
        }
    }
}

/// Delete every server's patch cache in this game folder (Settings, Reset
/// patched files).
pub fn clear_all(root: &Path) -> io::Result<()> {
    match fs::remove_dir_all(root.join(CACHE_DIR)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hv-patch-cache-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sha(bytes: &[u8]) -> String {
        format!("{:x}", sha2::Sha256::digest(bytes))
    }

    #[test]
    fn keys_from_manifest_hashes() {
        let h = sha(b"x");
        assert_eq!(cache_key(&h.to_uppercase(), "a.dll"), Some(h.clone()));
        let a = cache_key("DEADBEEF", "a.dll").unwrap();
        let b = cache_key("deadbeef", "dat/b.pac").unwrap();
        assert!(a.starts_with("crc32-deadbeef-"));
        assert_ne!(a, b, "a CRC32 key is per path");
        assert_eq!(cache_key("xyz", "a"), None);
        assert_eq!(cache_key("../../etc/passwd", "a"), None);
        assert_eq!(cache_key("abc", "a"), None);
    }

    fn version(etag: &str, channel: &str, last_used: u64, files: &[&str]) -> CachedVersion {
        CachedVersion {
            etag: etag.into(),
            channel: channel.into(),
            last_used,
            files: files.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn cleanup_keeps_installed_and_other_channels_newest() {
        let dir = scratch("prune");
        let files = dir.join(FILES_DIR);
        fs::create_dir_all(&files).unwrap();
        for (name, size) in [("s1", 10), ("s2", 10), ("s3", 10), ("p1", 10), ("p2", 10), ("shared", 10), ("stray", 1), ("x.part", 1)] {
            fs::write(files.join(name), vec![b'x'; size]).unwrap();
        }
        let mut index = Index {
            seq: 6,
            versions: vec![
                version("S1", "stable", 1, &["s1", "shared"]),
                version("P1", "pre:mods", 2, &["p1", "shared"]),
                version("S2", "stable", 3, &["s2", "shared"]),
                version("P2", "pre:mods", 4, &["p2", "shared"]),
                version("S3", "stable", 6, &["s3", "shared"]),
            ],
        };
        // Count: 3 versions -> S1 (oldest, not protected) and P1 go; S3 is
        // installed, P2 the newest pre-release, S2 the next newest.
        prune(&dir, &mut index, "S3", 3, u64::MAX);
        let etags: Vec<&str> = index.versions.iter().map(|v| v.etag.as_str()).collect();
        assert_eq!(etags, ["S3", "P2", "S2"]);
        for gone in ["s1", "p1", "stray", "x.part"] {
            assert!(!files.join(gone).exists(), "{gone} should be removed");
        }
        for kept in ["s2", "s3", "p2", "shared"] {
            assert!(files.join(kept).exists(), "{kept} should be kept");
        }

        // Size: 40 bytes kept, cap 30 -> S2 (rank 2) goes first; cap 5 -> then
        // P2 (other channel's newest); the installed S3 stays even above the cap.
        prune(&dir, &mut index, "S3", 3, 30);
        let etags: Vec<&str> = index.versions.iter().map(|v| v.etag.as_str()).collect();
        assert_eq!(etags, ["S3", "P2"]);
        prune(&dir, &mut index, "S3", 3, 5);
        let etags: Vec<&str> = index.versions.iter().map(|v| v.etag.as_str()).collect();
        assert_eq!(etags, ["S3"]);
        assert!(files.join("s3").exists() && files.join("shared").exists());
        assert!(!files.join("p2").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn remember_replaces_same_etag_and_clear_all() {
        let root = scratch("remember");
        let server = "127.0.0.1:8080";
        let h = sha(b"a");
        remember_installed(&root, server, "\"E1\"", "stable", &[(h.clone(), "a.dll".into())]);
        remember_installed(&root, server, "\"E2\"", "pre:mods", &[]);
        remember_installed(&root, server, "\"E1\"", "stable", &[(h.clone(), "a.dll".into())]);
        let index = load_index(&server_dir(&root, server));
        assert_eq!(index.versions.len(), 2);
        assert_eq!(index.versions[0].etag, "\"E1\"");
        assert_eq!(index.versions[0].last_used, 3);
        assert_eq!(index.versions[0].files, vec![h]);
        assert!(server_dir(&root, server).ends_with("PatchCache/127.0.0.1_8080"));
        clear_all(&root).unwrap();
        assert!(!root.join(CACHE_DIR).exists());
        clear_all(&root).unwrap();
        fs::remove_dir_all(&root).unwrap();
    }
}
