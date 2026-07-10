//! Per-root incremental index cache.
//!
//! Wraps `build_index`'s walk → parse → assemble pipeline with a refresh
//! algorithm that reuses unchanged files' parse output instead of
//! re-indexing the whole root on every call:
//!
//! 1. `walk_sources(root)`.
//! 2. Per file, stat (`mtime` + `size`): unchanged vs. the stored entry ⇒
//!    reuse the stored `FilePartial` outright (no I/O beyond the `stat`).
//! 3. Stat changed (or file is new) ⇒ `compute_file_hash`. Hash equal to the
//!    stored one ⇒ the content didn't actually change (e.g. a touch, or a
//!    save that round-trips to the same bytes) — refresh the stored stat and
//!    keep the old `FilePartial`. Hash differs ⇒ queue for re-parse.
//! 4. Files that dropped out of the walk (deleted) are dropped from the
//!    cache.
//! 5. Queued files are re-parsed in parallel via `index_one_file`.
//! 6. If anything changed (first build, a re-parse, or a deletion), the
//!    `Index` is re-assembled from every current `FilePartial` (via the same
//!    `assemble_index` `build_index` uses, so the result is
//!    indistinguishable from a fresh `build_index` call) and stored as a new
//!    `Arc<Index>`. Otherwise the previously stored `Arc` is returned
//!    untouched — callers can `Arc::ptr_eq` to detect a true no-op refresh.
//!
//! `CodeIntelCache` is `Send + Sync` (a `Mutex` around the whole per-root
//! map). Holding the mutex for the full duration of a refresh — including
//! the parallel re-parse — is a deliberate v1 simplification: callers are
//! already expected to invoke `get_index` from `spawn_blocking`, and a
//! second caller racing a refresh of a *different* root simply queues
//! behind it rather than proceeding concurrently. Correctness over
//! throughput for the first cut.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use rayon::prelude::*;

use crate::hash::{compute_file_hash, FileHash};
use crate::index::{
    assemble_index, compile_queries, index_one_file, FilePartial, Index, LangQueries,
};
use crate::lang::Language;
use crate::walk::{walk_sources, SourceFile};

/// Maximum number of roots kept warm at once. Past this, the least recently
/// touched root is evicted (its next `get_index` call rebuilds from scratch).
const CAPACITY: usize = 8;

/// One cached file's parse output plus the stat/hash used to detect drift.
/// `partial: None` is a parse-failure marker (unreadable file, grammar
/// failure, or parse failure) — kept so `assemble_index` can still surface a
/// `warnings` entry for it without needing to distinguish "never seen" from
/// "seen but unparseable".
struct CachedFileEntry {
    mtime: SystemTime,
    size: u64,
    hash: FileHash,
    partial: Option<FilePartial>,
}

/// Everything cached for one root: the per-file entries (keyed by relative
/// path, matching `Index`'s own keying) and the last assembled `Index`.
struct CachedRoot {
    files: HashMap<String, CachedFileEntry>,
    index: Arc<Index>,
}

struct CacheInner {
    roots: HashMap<PathBuf, CachedRoot>,
    /// Touch order, oldest first. The front is evicted when `roots` grows
    /// past `CAPACITY`.
    lru: Vec<PathBuf>,
}

/// Per-root incremental index cache. See module docs for the refresh
/// algorithm. `new()` allocates capacity for `CAPACITY` roots; callers don't
/// need to size it.
pub struct CodeIntelCache {
    inner: Mutex<CacheInner>,
}

impl Default for CodeIntelCache {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeIntelCache {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(CacheInner {
                roots: HashMap::new(),
                lru: Vec::new(),
            }),
        }
    }

    /// Return the current `Index` for `root`, refreshing it first. Returns
    /// the same `Arc` (by pointer) as the previous call when nothing on disk
    /// changed since then.
    pub fn get_index(&self, root: &Path) -> anyhow::Result<Arc<Index>> {
        let root_key = root.to_path_buf();
        let files = walk_sources(root)?;

        let mut guard = self.inner.lock().unwrap();
        let is_new_root = !guard.roots.contains_key(&root_key);
        let mut any_change = is_new_root;
        let mut seen_rels: HashSet<String> = HashSet::with_capacity(files.len());

        // Pass 1: stat (and, if needed, hash) every file in the current walk
        // against what's stored. Doesn't mutate the cache yet — just sorts
        // files into "reuse as-is", "refresh stat only", or "needs reparse".
        struct ReparseItem {
            rel: String,
            path: PathBuf,
            language: Language,
            mtime: SystemTime,
            size: u64,
            hash: FileHash,
        }
        let mut to_reparse: Vec<ReparseItem> = Vec::new();
        let mut stat_refresh: Vec<(String, SystemTime, u64)> = Vec::new();

        {
            let existing = guard.roots.get(&root_key);
            for f in &files {
                let rel = f
                    .path
                    .strip_prefix(root)
                    .unwrap_or(&f.path)
                    .to_string_lossy()
                    .into_owned();
                // Stat (and, if needed, hash) failures here are treated as "the
                // file is gone" rather than propagated: `walk_sources` already
                // ran and listed this file, but under a TOCTOU race (deleted or
                // replaced between the walk and this stat) the I/O can fail even
                // though nothing is actually wrong with the cache or the rest of
                // the walk. Matching `build_index`'s per-file graceful-degrade
                // behaviour, we simply don't mark this file "seen" — the
                // deletion pass below (which drops any stored entry not in
                // `seen_rels`) then removes it like any other vanished file,
                // instead of discarding the whole refresh (and a good cached
                // `Arc`) over one file's benign disappearance. Errors from the
                // walk itself (above, `walk_sources(root)?`) are NOT covered by
                // this and remain hard errors.
                let Ok(meta) = fs::metadata(&f.path) else {
                    continue;
                };
                let Ok(mtime) = meta.modified() else {
                    continue;
                };
                let size = meta.len();

                let stored = existing.and_then(|r| r.files.get(&rel));
                if let Some(e) = stored {
                    if e.mtime == mtime && e.size == size {
                        // Fast path: stat identical, nothing to do.
                        seen_rels.insert(rel.clone());
                        continue;
                    }
                }

                let Ok(hash) = compute_file_hash(&f.path) else {
                    continue;
                };
                seen_rels.insert(rel.clone());
                if let Some(e) = stored {
                    if e.hash == hash {
                        // Content round-tripped to the same bytes — stat moved
                        // but there's nothing new to parse.
                        stat_refresh.push((rel, mtime, size));
                        continue;
                    }
                }

                any_change = true;
                to_reparse.push(ReparseItem {
                    rel,
                    path: f.path.clone(),
                    language: f.language,
                    mtime,
                    size,
                    hash,
                });
            }
        }

        // Pass 2: re-parse the changed set in parallel. Compile queries only
        // for the languages actually present in `to_reparse`.
        let mut qcache: HashMap<Language, LangQueries> = HashMap::new();
        for item in &to_reparse {
            if let std::collections::hash_map::Entry::Vacant(e) = qcache.entry(item.language) {
                e.insert(compile_queries(item.language)?);
            }
        }
        let reparsed: Vec<(&ReparseItem, Option<FilePartial>)> = to_reparse
            .par_iter()
            .map(|item| {
                let sf = SourceFile {
                    path: item.path.clone(),
                    language: item.language,
                };
                let partial = qcache
                    .get(&item.language)
                    .and_then(|q| index_one_file(&sf, root, q));
                (item, partial)
            })
            .collect();

        // Pass 3: apply everything to the stored root entry.
        let result = {
            let root_entry = guard
                .roots
                .entry(root_key.clone())
                .or_insert_with(|| CachedRoot {
                    files: HashMap::new(),
                    index: Arc::new(Index::default()),
                });

            for (rel, mtime, size) in stat_refresh {
                if let Some(e) = root_entry.files.get_mut(&rel) {
                    e.mtime = mtime;
                    e.size = size;
                }
            }
            for (item, partial) in reparsed {
                root_entry.files.insert(
                    item.rel.clone(),
                    CachedFileEntry {
                        mtime: item.mtime,
                        size: item.size,
                        hash: item.hash,
                        partial,
                    },
                );
            }

            let removed: Vec<String> = root_entry
                .files
                .keys()
                .filter(|rel| !seen_rels.contains(*rel))
                .cloned()
                .collect();
            if !removed.is_empty() {
                any_change = true;
                for rel in removed {
                    root_entry.files.remove(&rel);
                }
            }

            if any_change {
                let partials: Vec<(String, Option<FilePartial>)> = root_entry
                    .files
                    .iter()
                    .map(|(rel, e)| (rel.clone(), e.partial.clone()))
                    .collect();
                root_entry.index = Arc::new(assemble_index(partials));
            }

            Arc::clone(&root_entry.index)
        };

        // Touch LRU and evict past capacity.
        guard.lru.retain(|p| p != &root_key);
        guard.lru.push(root_key);
        while guard.lru.len() > CAPACITY {
            let evict = guard.lru.remove(0);
            guard.roots.remove(&evict);
        }

        Ok(result)
    }

    /// Drop the cached entries for `rel_paths` outright, forcing a re-parse
    /// on the next `get_index` regardless of stat. This is the only way to
    /// invalidate a same-mtime-same-size edit — the fast path in
    /// `get_index` can't see that case by design.
    pub fn invalidate_files(&self, root: &Path, rel_paths: &[String]) {
        let mut guard = self.inner.lock().unwrap();
        if let Some(r) = guard.roots.get_mut(&root.to_path_buf()) {
            for rel in rel_paths {
                r.files.remove(rel);
            }
        }
    }

    /// Drop the entire cached entry for `root`. The next `get_index` call
    /// rebuilds it from scratch.
    pub fn invalidate_root(&self, root: &Path) {
        let mut guard = self.inner.lock().unwrap();
        let key = root.to_path_buf();
        guard.roots.remove(&key);
        guard.lru.retain(|p| p != &key);
    }
}
