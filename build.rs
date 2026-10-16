use std::{env, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-changed=assets/pocketplay.rc");
    println!("cargo:rerun-if-changed=assets/pocketplay.ico");
    println!("cargo:rerun-if-changed=assets/pocketplay.manifest");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed=media/engine.c");
    println!("cargo:rerun-if-changed=media/symbols.h");
    let media = out.join("media-engine.o");
    let mut compiler = Command::new("clang.exe");
    if env::var_os("CARGO_FEATURE_QA").is_some() {
        compiler.arg("-DPOCKETPLAY_MEDIA_QA");
    }
    let status = compiler
        .args([
            "-O3",
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-I",
            "media/include",
            "-c",
            "media/engine.c",
            "-o",
        ])
        .arg(&media)
        .status();
    assert!(
        status.expect("Install the native Clang compiler").success(),
        "Native media ABI glue failed to compile"
    );
    println!("cargo:rustc-link-arg={}", media.display());
    // Add only the missing SDK import. Rust's bundled GNU CRT must remain
    // together; mixing it with LLVM's newer UCRT startup libraries is invalid.
    if let Some(path) = env::var_os("POCKETPLAY_MINGW_LIB") {
        let imports = out.join("sdk-imports");
        std::fs::create_dir_all(&imports).unwrap();
        std::fs::copy(
            PathBuf::from(path).join("libshlwapi.a"),
            imports.join("libshlwapi.a"),
        )
        .unwrap();
        println!("cargo:rustc-link-search=native={}", imports.display());
    }
    let gnu = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default() == "gnu";
    let output = out.join(if gnu {
        "pocketplay.o"
    } else {
        "pocketplay.res"
    });
    let status = if gnu {
        Command::new("windres.exe")
            .args(["-i", "assets/pocketplay.rc", "-O", "coff", "-o"])
            .arg(&output)
            .status()
    } else {
        Command::new("rc.exe")
            .args(["/nologo", "/fo"])
            .arg(&output)
            .arg("assets/pocketplay.rc")
            .status()
    };
    assert!(
        status
            .expect(
                "Install a Windows resource compiler (windres for GNU, Windows SDK rc for MSVC)"
            )
            .success(),
        "Windows resource compilation failed."
    );
    println!("cargo:rustc-link-arg={}", output.display());
}
