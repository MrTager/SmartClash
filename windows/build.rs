//! 两件事：
//!
//! 1. 链接占位库。Rust 自带的 self-contained MinGW 不附带 wininet 导入库，而
//!    eframe → winapi 的默认特性会向链接器传 `-lwininet`。本程序没有直接调用
//!    wininet 符号，因此用一个空的占位归档满足该链接参数；如果将来真的用到
//!    wininet API，链接会明确报出缺失符号，而不是静默失效。
//!
//! 2. 把 `resources/app.ico` 与版本信息写进 exe（任务栏 / 资源管理器 / 文件属性）。
//!    资源编译依赖 windres（MSVC 工具链则是 rc.exe）；缺少时只告警，不中断构建。

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".linker");
        let archive = dir.join("libwininet.a");
        if !archive.exists() {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(&archive, b"!<arch>\n");
        }
        println!("cargo:rustc-link-search=native={}", dir.display());
        println!("cargo:rerun-if-changed={}", archive.display());
    }

    embed_windows_resources();
    pin_bundled_cores();
}

#[cfg(windows)]
fn embed_windows_resources() {
    let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/app.ico");
    if !icon.exists() {
        println!("cargo:warning=未找到 resources/app.ico，跳过图标嵌入");
        return;
    }
    println!("cargo:rerun-if-changed={}", icon.display());

    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let mut res = winres::WindowsResource::new();
    res.set_icon(icon.to_str().unwrap_or("resources/app.ico"));
    res.set("FileDescription", "SmartClash");
    res.set("ProductName", "SmartClash");
    res.set("OriginalFilename", "SmartClash.exe");
    res.set("InternalName", "SmartClash");
    res.set("CompanyName", "SmartClash");
    res.set("LegalCopyright", "GPL-3.0");
    res.set("FileVersion", &format!("{version}.0"));
    res.set("ProductVersion", &format!("{version}.0"));

    if let Err(error) = res.compile() {
        // 没有 windres / rc.exe 的环境不该因为图标而构建失败
        println!("cargo:warning=图标资源编译失败（不影响程序功能）：{error}");
        return;
    }

    // GNU ld 不会从静态库里捞出"不定义任何符号"的资源对象，于是 -l resource 会被
    // 完全忽略（exe 里就没有 .rsrc 段）。直接把 resource.o 作为链接输入喂进去。
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
        if let Ok(out) = std::env::var("OUT_DIR") {
            let obj = std::path::Path::new(&out).join("resource.o");
            if obj.exists() {
                println!("cargo:rustc-link-arg={}", obj.display());
            }
        }
    }
}

#[cfg(not(windows))]
fn embed_windows_resources() {}

#[cfg(windows)]
fn pin_bundled_cores() {
    use sha2::{Digest, Sha256};
    for (name, variable) in [
        ("mihomo.exe", "SMARTCLASH_MIHOMO_HASH"),
        ("sing-box.exe", "SMARTCLASH_SINGBOX_HASH"),
        ("xray.exe", "SMARTCLASH_XRAY_HASH"),
        ("wintun.dll", "SMARTCLASH_WINTUN_HASH"),
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        let data = std::fs::read(&path).expect("固定 bundled 内核资源必须存在");
        println!("cargo:rustc-env={variable}={:x}", Sha256::digest(data));
    }
}
#[cfg(not(windows))]
fn pin_bundled_cores() {}
