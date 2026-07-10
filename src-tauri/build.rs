use std::fs;
use std::path::PathBuf;

fn main() {
    // Create zero-byte rtk placeholder so cargo check passes on bare checkouts.
    // tauri-build validates externalBin eagerly; this sentinel allows plain builds
    // before fetch-rtk.sh has staged the real binary.
    let target = std::env::var("TARGET").expect("TARGET not set by cargo");
    let rtk_path = PathBuf::from(format!("binaries/rtk-{}", target));

    if let Some(parent) = rtk_path.parent() {
        fs::create_dir_all(parent).expect("failed to create binaries directory");
    }

    if !rtk_path.exists() {
        fs::File::create(&rtk_path).expect("failed to create rtk placeholder");
    }

    // Release builds must never ship the zero-byte placeholder silently: the
    // bundler would copy it into the app as a broken rtk. Fail closed here —
    // this is the one choke point every bundle passes through, regardless of
    // entry point (pnpm tauri build, direct cargo tauri build, cross-compile).
    println!("cargo:rerun-if-env-changed=CONCLAVE_RTK_PLACEHOLDER_OK");
    let profile = std::env::var("PROFILE").unwrap_or_default();
    let staged_len = fs::metadata(&rtk_path).map(|m| m.len()).unwrap_or(0);
    let placeholder_ok = std::env::var("CONCLAVE_RTK_PLACEHOLDER_OK").as_deref() == Ok("1");
    if profile == "release" && staged_len == 0 && !placeholder_ok {
        panic!(
            "binaries/rtk-{target} is the zero-byte placeholder; a release build would \
             silently bundle a broken rtk. Stage the real binary first: run \
             `bash scripts/fetch-rtk.sh` (pnpm tauri build does this automatically), or \
             stage src-tauri/binaries/rtk-{target} yourself when cross-compiling. Set \
             CONCLAVE_RTK_PLACEHOLDER_OK=1 to intentionally build a release without rtk."
        );
    }

    tauri_build::build()
}
