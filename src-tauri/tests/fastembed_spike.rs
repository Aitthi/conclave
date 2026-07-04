//! T1 prototype-gate spike — NOT shipped code.
//!
//! Proves the load-bearing assumptions in docs/2026-07-04-plan-workspace-memory-v1.md
//! T1 before anything merges: fastembed 5.17 / AllMiniLML6V2Q embeds text
//! locally, the model cache honors a Conclave-scoped directory override
//! (not the crate's default `./.fastembed_cache`), and a warm cache runs
//! fully offline. Both tests are `#[ignore]`d — run manually, in order:
//!
//! ```sh
//! cargo test --release --manifest-path src-tauri/Cargo.toml \
//!     --test fastembed_spike -- --ignored --nocapture --test-threads=1
//! ```

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

/// Mirrors the app-support convention in `engine/db.rs::db_path`
/// (`dirs::data_dir()` + `Conclave`), scoped to a `models` subdir so the
/// DB file and model cache don't collide.
fn conclave_model_cache_dir() -> std::path::PathBuf {
    dirs::data_dir()
        .expect("could not resolve user data directory")
        .join("Conclave")
        .join("models")
}

#[test]
#[ignore = "downloads ~23MB model + onnxruntime dylib on first run (network required); run manually"]
fn embeds_three_strings_with_conclave_cache_dir_override() {
    let cache_dir = conclave_model_cache_dir();
    std::fs::create_dir_all(&cache_dir).expect("create cache dir");

    let options = TextInitOptions::new(EmbeddingModel::AllMiniLML6V2Q)
        .with_cache_dir(cache_dir.clone())
        .with_show_download_progress(true);

    let mut model =
        TextEmbedding::try_new(options).expect("model init failed (needs network on first run)");

    let texts = vec![
        "the quick brown fox",
        "jumps over the lazy dog",
        "workspace memory system",
    ];

    let embeddings = model.embed(texts.clone(), None).expect("embed");

    assert_eq!(embeddings.len(), 3);
    let dim = embeddings[0].len();
    println!("[T1 spike] dimension: {dim}");
    for (text, emb) in texts.iter().zip(&embeddings) {
        let head = &emb[..4.min(emb.len())];
        println!("[T1 spike] {text:?} -> first 4 values: {head:?}");
    }

    // Cache-dir override proof: files must land under our chosen dir, not
    // the crate's default `./.fastembed_cache` (cwd-relative — unacceptable
    // for a packaged app, per the plan's risk ledger).
    let cache_has_files = std::fs::read_dir(&cache_dir)
        .expect("read cache dir")
        .next()
        .is_some();
    assert!(
        cache_has_files,
        "expected model files under {cache_dir:?}; cache-dir override did not take effect"
    );

    let default_cache = std::path::Path::new(".fastembed_cache");
    assert!(
        !default_cache.exists(),
        "model leaked into default ./.fastembed_cache despite cache_dir override"
    );
    println!("[T1 spike] cache-dir override confirmed: {cache_dir:?}");
}

#[test]
#[ignore = "run AFTER embeds_three_strings_with_conclave_cache_dir_override has populated the cache; disconnect network first"]
fn second_run_is_offline() {
    // Manual verification (not automated network-kill, run by hand):
    //   1. Run `embeds_three_strings_with_conclave_cache_dir_override` once (online).
    //   2. Disconnect network (Wi-Fi off / airplane mode).
    //   3. Re-run this test — it must succeed without touching the network,
    //      proving the model is fully cached and the app is offline-capable
    //      after first use (plan risk ledger: "hf.co reachability at first use").
    let cache_dir = conclave_model_cache_dir();
    let options = TextInitOptions::new(EmbeddingModel::AllMiniLML6V2Q)
        .with_cache_dir(cache_dir)
        .with_show_download_progress(false);

    let mut model = TextEmbedding::try_new(options).expect("model init from cache, offline");
    let embeddings = model.embed(vec!["offline check"], None).expect("embed offline");
    assert_eq!(embeddings.len(), 1);
    println!("[T1 spike] offline embed OK, dim={}", embeddings[0].len());
}

#[test]
#[ignore = "downloads model on first run; observes which ONNX execution provider actually initializes"]
fn observes_execution_provider() {
    // fastembed's default `execution_providers` list is empty, which ORT
    // resolves to its built-in CPU EP. This is the plan's guaranteed
    // baseline (docs/2026-07-04-mempalace-rust-port-scope.md section 2:
    // "Treat CPU as the guaranteed baseline and CoreML as an optimization
    // gate"). Registering `CoreMLExecutionProvider` requires depending on
    // `ort` directly (not re-exported by fastembed) plus its `coreml`
    // cargo feature — deliberately deferred out of this fast gate per the
    // scope report's own recommendation; CoreML assignment needs a
    // dedicated empirical provider-assignment + latency spike later.
    let cache_dir = conclave_model_cache_dir();
    std::fs::create_dir_all(&cache_dir).expect("create cache dir");

    let cpu_options = TextInitOptions::new(EmbeddingModel::AllMiniLML6V2Q)
        .with_cache_dir(cache_dir)
        .with_show_download_progress(false);
    let mut cpu_model = TextEmbedding::try_new(cpu_options).expect("CPU EP model init");
    let cpu_out = cpu_model.embed(vec!["cpu execution provider check"], None);
    println!(
        "[T1 spike] CPU EP (default, no explicit provider) embed ok: {}",
        cpu_out.is_ok()
    );
    assert!(cpu_out.is_ok(), "CPU EP (the guaranteed baseline) must always work");
}
