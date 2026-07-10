//! The four codegraph verbs (find-refs/callers/callees/impact), re-implemented
//! as library functions over the shared [`crate::index::Index`] instead of
//! codegraph's own CLI commands.
//!
//! Every function here returns the *inner* `data` value (`anyhow::Result<Value>`,
//! or `(Value, bool)` where the bool means "results were truncated by `limit`",
//! matching [`crate::map`]'s convention). Wrapping in the wire envelope
//! (`output::envelope`) is the engine's job, not this module's — these are
//! library calls, not CLI commands.

use crate::index::{DefKind, Index, RefKind};
use crate::resolve::resolve_refs;
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet, VecDeque};

/// Cap on `callers`/`callees` BFS hops — guards against pathological fan-out
/// graphs (e.g. a widely-called `new`/`log` style helper).
const HARD_CAP: usize = 8;
/// Cap on `impact` BFS hops. Shallower than `HARD_CAP` since impact fans out
/// over every reference kind (not just calls), so it grows faster per hop.
const MAX_DEPTH: usize = 6;

/// Array of `{file,line,column,kind,name,context,confidence,reason}`, `kind` ∈
/// `definition|call|reference`, sorted by `(file,line)`. `limit` truncates
/// *after* the sort, matching [`crate::map::find`]'s convention.
pub fn find_refs(idx: &Index, name: &str, limit: usize) -> Result<(Value, bool)> {
    struct Hit {
        file: String,
        line: usize,
        column: usize,
        kind: &'static str,
        name: String,
        context: String,
        confidence: &'static str,
        reason: &'static str,
    }

    let mut hits = Vec::new();
    for d in &idx.definitions {
        if d.name == name {
            hits.push(Hit {
                file: d.file.clone(),
                line: d.line,
                column: d.column,
                kind: "definition",
                name: d.name.clone(),
                context: format!("{:?} {}", d.kind, d.name).to_lowercase(),
                confidence: "high",
                reason: "same-file-scope",
            });
        }
    }
    for r in resolve_refs(idx, name) {
        let kind = match r.reference.kind {
            RefKind::Call => "call",
            RefKind::Reference => "reference",
        };
        hits.push(Hit {
            file: r.reference.file.clone(),
            line: r.reference.line,
            column: r.reference.column,
            kind,
            name: r.reference.name.clone(),
            context: r.reference.context.clone(),
            confidence: r.confidence.as_str(),
            reason: r.reason.as_str(),
        });
    }
    hits.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));

    let total = hits.len();
    let truncated = total > limit;
    let arr: Vec<Value> = hits
        .into_iter()
        .take(limit)
        .map(|h| {
            json!({
                "file": h.file,
                "line": h.line,
                "column": h.column,
                "kind": h.kind,
                "name": h.name,
                "context": h.context,
                "confidence": h.confidence,
                "reason": h.reason,
            })
        })
        .collect();
    Ok((Value::Array(arr), truncated))
}

struct CallSite {
    file: String,
    line: usize,
    column: usize,
    context: String,
}

fn call_site_json(s: CallSite) -> Value {
    json!({
        "file": s.file,
        "line": s.line,
        "column": s.column,
        "context": s.context,
    })
}

/// Array of `{file,line,column,name,kind,distance,confidence,reason,sites}`,
/// sorted by `(distance,file,line)`. BFS walks call-references only, up to
/// `depth` hops (capped at [`HARD_CAP`]); unlimited element count, bounded by
/// the depth cap rather than a `limit` parameter.
pub fn callers(idx: &Index, name: &str, depth: usize) -> Result<Value> {
    let depth_limit = depth.min(HARD_CAP);

    struct CallerEntry {
        file: String,
        line: usize,
        column: usize,
        name: String,
        kind: &'static str,
        distance: usize,
        confidence: &'static str,
        reason: &'static str,
        sites: Vec<CallSite>,
    }

    let mut by_caller: BTreeMap<(String, String, usize), CallerEntry> = BTreeMap::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    queue.push_back((name.to_string(), 0));
    visited.insert(name.to_string());

    while let Some((current, dist)) = queue.pop_front() {
        if dist >= depth_limit {
            continue;
        }
        for r in resolve_refs(idx, &current) {
            if r.reference.kind != RefKind::Call {
                continue;
            }
            let Some(enclosing) =
                idx.enclosing_definition(&r.reference.file, r.reference.byte_offset)
            else {
                continue;
            };
            if !matches!(enclosing.kind, DefKind::Fn | DefKind::Method) {
                continue;
            }
            let key = (
                enclosing.name.clone(),
                enclosing.file.clone(),
                enclosing.line,
            );
            let entry = by_caller.entry(key).or_insert_with(|| CallerEntry {
                file: enclosing.file.clone(),
                line: enclosing.line,
                column: enclosing.column,
                name: enclosing.name.clone(),
                kind: if enclosing.kind == DefKind::Method {
                    "method"
                } else {
                    "fn"
                },
                distance: dist + 1,
                confidence: r.confidence.as_str(),
                reason: r.reason.as_str(),
                sites: Vec::new(),
            });
            entry.sites.push(CallSite {
                file: r.reference.file.clone(),
                line: r.reference.line,
                column: r.reference.column,
                context: r.reference.context.clone(),
            });
            if visited.insert(enclosing.name.clone()) {
                queue.push_back((enclosing.name.clone(), dist + 1));
            }
        }
    }

    let mut entries: Vec<CallerEntry> = by_caller.into_values().collect();
    entries.sort_by(|a, b| {
        a.distance
            .cmp(&b.distance)
            .then(a.file.cmp(&b.file))
            .then(a.line.cmp(&b.line))
    });

    let arr: Vec<Value> = entries
        .into_iter()
        .map(|e| {
            json!({
                "file": e.file,
                "line": e.line,
                "column": e.column,
                "name": e.name,
                "kind": e.kind,
                "distance": e.distance,
                "confidence": e.confidence,
                "reason": e.reason,
                "sites": e.sites.into_iter().map(call_site_json).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(Value::Array(arr))
}

/// Array of `{name,kind,def_file,def_line,distance,confidence,reason,sites}`,
/// sorted by `(distance,name)`. `def_file`/`def_line` are `null` (key always
/// present) when the callee's definition couldn't be resolved. BFS walks call
/// sites textually enclosed by known `fn`/`method` definitions, up to `depth`
/// hops (capped at [`HARD_CAP`]).
pub fn callees(idx: &Index, name: &str, depth: usize) -> Result<Value> {
    let depth_limit = depth.min(HARD_CAP);

    struct CalleeEntry {
        name: String,
        kind: &'static str,
        def_file: Option<String>,
        def_line: Option<usize>,
        distance: usize,
        confidence: &'static str,
        reason: &'static str,
        sites: Vec<CallSite>,
    }

    let mut entries: BTreeMap<String, CalleeEntry> = BTreeMap::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    queue.push_back((name.to_string(), 0));
    visited.insert(name.to_string());

    while let Some((current, dist)) = queue.pop_front() {
        if dist >= depth_limit {
            continue;
        }
        let outer: Vec<_> = idx
            .definitions
            .iter()
            .filter(|d| d.name == current && matches!(d.kind, DefKind::Fn | DefKind::Method))
            .collect();
        if outer.is_empty() {
            continue;
        }
        for r in &idx.references {
            if r.kind != RefKind::Call {
                continue;
            }
            if !outer.iter().any(|o| {
                o.file == r.file
                    && o.body_start_byte <= r.byte_offset
                    && r.byte_offset < o.body_end_byte
            }) {
                continue;
            }
            let resolutions = resolve_refs(idx, &r.name);
            let chosen = resolutions.iter().find(|res| {
                res.reference.byte_offset == r.byte_offset && res.reference.file == r.file
            });
            let (confidence, reason, def_file, def_line) = match chosen {
                Some(res) => (
                    res.confidence.as_str(),
                    res.reason.as_str(),
                    res.definition.map(|d| d.file.clone()),
                    res.definition.map(|d| d.line),
                ),
                None => ("low", "name-only", None, None),
            };
            let entry = entries.entry(r.name.clone()).or_insert(CalleeEntry {
                name: r.name.clone(),
                kind: "fn",
                def_file: def_file.clone(),
                def_line,
                distance: dist + 1,
                confidence,
                reason,
                sites: Vec::new(),
            });
            entry.sites.push(CallSite {
                file: r.file.clone(),
                line: r.line,
                column: r.column,
                context: r.context.clone(),
            });
            if visited.insert(r.name.clone()) {
                queue.push_back((r.name.clone(), dist + 1));
            }
        }
    }

    let mut out: Vec<CalleeEntry> = entries.into_values().collect();
    out.sort_by(|a, b| a.distance.cmp(&b.distance).then(a.name.cmp(&b.name)));

    let arr: Vec<Value> = out
        .into_iter()
        .map(|e| {
            json!({
                "name": e.name,
                "kind": e.kind,
                "def_file": e.def_file,
                "def_line": e.def_line,
                "distance": e.distance,
                "confidence": e.confidence,
                "reason": e.reason,
                "sites": e.sites.into_iter().map(call_site_json).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(Value::Array(arr))
}

/// Maps a `DefKind` to the lowercase string codegraph/consumers expect.
/// Mirrors `crate::map::kind_str` (kept local — `map`'s copy isn't `pub`).
fn kind_label(k: DefKind) -> &'static str {
    match k {
        DefKind::Fn => "fn",
        DefKind::Struct => "struct",
        DefKind::Enum => "enum",
        DefKind::Trait => "trait",
        DefKind::Class => "class",
        DefKind::Interface => "interface",
        DefKind::Type => "type",
        DefKind::Const => "const",
        DefKind::Method => "method",
    }
}

/// Array of `{name,kind,file,line,distance,confidence,reason}`, sorted by
/// `(distance,file,line)`. Seeds at every definition matching `name`
/// (distance=0), then BFS over every reference kind up to [`MAX_DEPTH`] hops,
/// recursing only through call-kind references into `fn`/`method` bodies
/// (type-position uses don't propagate impact further).
pub fn impact(idx: &Index, name: &str) -> Result<Value> {
    struct ImpactEntry {
        name: String,
        kind: &'static str,
        file: String,
        line: usize,
        distance: usize,
        confidence: &'static str,
        reason: &'static str,
    }

    let mut entries: BTreeMap<(String, String, usize), ImpactEntry> = BTreeMap::new();
    // Seed: the symbol itself (every matching definition, distance=0).
    for d in idx.definitions.iter().filter(|d| d.name == name) {
        entries.insert(
            (d.name.clone(), d.file.clone(), d.line),
            ImpactEntry {
                name: d.name.clone(),
                kind: kind_label(d.kind),
                file: d.file.clone(),
                line: d.line,
                distance: 0,
                confidence: "high",
                reason: "same-file-scope",
            },
        );
    }

    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    let mut visited: HashSet<String> = HashSet::new();
    queue.push_back((name.to_string(), 0));
    visited.insert(name.to_string());

    while let Some((current, dist)) = queue.pop_front() {
        if dist >= MAX_DEPTH {
            continue;
        }
        for resolved in resolve_refs(idx, &current) {
            let Some(enclosing) =
                idx.enclosing_definition(&resolved.reference.file, resolved.reference.byte_offset)
            else {
                continue;
            };
            let key = (
                enclosing.name.clone(),
                enclosing.file.clone(),
                enclosing.line,
            );
            entries.entry(key).or_insert(ImpactEntry {
                name: enclosing.name.clone(),
                kind: kind_label(enclosing.kind),
                file: enclosing.file.clone(),
                line: enclosing.line,
                distance: dist + 1,
                confidence: resolved.confidence.as_str(),
                reason: resolved.reason.as_str(),
            });
            // Recurse on call-kind references only — type-position uses don't propagate impact.
            if resolved.reference.kind == RefKind::Call
                && matches!(enclosing.kind, DefKind::Fn | DefKind::Method)
                && visited.insert(enclosing.name.clone())
            {
                queue.push_back((enclosing.name.clone(), dist + 1));
            }
        }
    }

    let mut out: Vec<ImpactEntry> = entries.into_values().collect();
    out.sort_by(|a, b| {
        a.distance
            .cmp(&b.distance)
            .then(a.file.cmp(&b.file))
            .then(a.line.cmp(&b.line))
    });

    let arr: Vec<Value> = out
        .into_iter()
        .map(|e| {
            json!({
                "name": e.name,
                "kind": e.kind,
                "file": e.file,
                "line": e.line,
                "distance": e.distance,
                "confidence": e.confidence,
                "reason": e.reason,
            })
        })
        .collect();
    Ok(Value::Array(arr))
}
