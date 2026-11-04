//! Rust 自带的 self-contained MinGW 不附带 wininet 导入库，而 eframe → winapi 的
//! 默认特性会向链接器传 `-lwininet`。本程序没有直接调用 wininet 符号，因此用一个
//! 空的占位归档满足该链接参数；如果将来真的用到 wininet API，链接会明确报出缺失
//! 符号，而不是静默失效。
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("gnu") {
        return;
    }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".linker");
    let archive = dir.join("libwininet.a");
    if !archive.exists() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(&archive, b"!<arch>\n");
    }
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!("cargo:rerun-if-changed={}", archive.display());
}
