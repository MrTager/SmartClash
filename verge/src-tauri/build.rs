fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rustc-env=SMARTCLASH_BUILD_TARGET={}", std::env::var("TARGET")?);
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu")
        && std::env::var_os("CARGO_FEATURE_CLIPPY").is_some()
    {
        let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").ok_or("OUT_DIR is not set")?);
        let manifest = out.join("test.manifest");
        std::fs::write(
            &manifest,
            r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0"><dependency><dependentAssembly><assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/></dependentAssembly></dependency></assembly>"#,
        )?;
        let resource = out.join("test.rc");
        std::fs::write(
            &resource,
            format!("1 24 \"{}\"", manifest.display().to_string().replace('\\', "/")),
        )?;
        let object = out.join("test-manifest.o");
        let status = std::process::Command::new("windres")
            .arg(&resource)
            .arg(&object)
            .status()?;
        if !status.success() {
            return Err("unable to compile the Windows test manifest".into());
        }
        println!("cargo:rustc-link-arg={}", object.display());
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // Tauri's resource is linked to the app binary, but not to Rust unit-test harnesses or examples.
        // Give non-app targets their own manifest so tests and examples can resolve TaskDialogIndirect.
        let out_dir = std::env::var_os("OUT_DIR").ok_or_else(|| std::io::Error::other("OUT_DIR is not set"))?;
        let manifest_path = std::path::PathBuf::from(out_dir).join("windows-test-manifest.xml");
        std::fs::write(
            &manifest_path,
            r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity
        type="win32"
        name="Microsoft.Windows.Common-Controls"
        version="6.0.0.0"
        processorArchitecture="*"
        publicKeyToken="6595b64144ccf1df"
        language="*"
      />
    </dependentAssembly>
  </dependency>
</assembly>
"#,
        )?;
        let manifest_display = manifest_path.display();
        if std::env::var_os("CARGO_FEATURE_CLIPPY").is_some() {
            println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
            println!("cargo:rustc-link-arg=/MANIFESTINPUT:{manifest_display}");
        } else {
            println!("cargo:rustc-link-arg-examples=/MANIFEST:EMBED");
            println!("cargo:rustc-link-arg-examples=/MANIFESTINPUT:{manifest_display}");
        }
    }

    #[cfg(feature = "clippy")]
    {
        println!("cargo:warning=Skipping tauri_build during Clippy");
    }

    #[cfg(not(feature = "clippy"))]
    tauri_build::build();

    Ok(())
}
