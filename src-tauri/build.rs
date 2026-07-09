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

    tauri_build::build()
}
