# Building PocketPlay

This repository contains source code, not a complete binary distribution. Both Cargo.lock files are retained. SDKs, compilers, DLLs, and generated Android files are not committed.

## Build dependencies

- Windows x64 and PowerShell.
- Rust stable with the `x86_64-pc-windows-gnu` and `aarch64-linux-android` targets.
- LLVM MinGW with Clang, windres, and the x64 MinGW import libraries.
- Android NDK r28c (28.2.13676358), SDK Platform 35, and Build Tools 35.0.0.
- A JDK supporting `javac --release 11`; set `JAVA_HOME` to its installation directory.

Public FFmpeg and SDL3 headers required by `media/engine.c` are retained in `media/include`. Their presence does not provide the runtime libraries.

## Current helper-script layout

The existing PowerShell helpers use a development workspace layout. They are not an automatic dependency installer. Place the checkout two directories below the workspace root, or adapt the dependency paths at the top of the scripts to your machine:

```text
workspace/
  projects/
    PocketPlay/                    Repository checkout
  work/
    rust-toolchain/
      cargo/bin/cargo.exe
      rustup/toolchains/stable-x86_64-pc-windows-gnu/
      llvm-mingw-20261006-ucrt-x86_64/
    native-sdk/
      android-ndk-r28c/
      android-35/android.jar
      android-15/lib/d8.jar
```

`android-15` is the directory name of the extracted Build Tools 35.0.0 package in this layout; it does not mean API level 15. `android-35` contains SDK Platform 35. The Windows helper prepares LLVM MinGW and Rust's GNU linker paths. The Android helper configures the ARM64 NDK linker.

## Build

From the repository directory, after installing/configuring the dependencies:

```powershell
$env:JAVA_HOME = 'C:\path\to\your\jdk'
.\build-device.ps1
.\build.ps1
```

The Android helper produces `tools/native/libpocketplay_device.so` and `tools/native/pocketplay-launcher.jar`. The Windows executable is `target/release/pocketplay.exe`.

The desktop helper can use Cargo from PATH when its workspace-local toolchain is absent, but then you must configure a compatible compiler, linker, resource compiler, and `POCKETPLAY_MINGW_LIB` yourself. Setting `JAVA_HOME` alone does not configure SDK/NDK paths.

## Runtime distribution

Create this directory structure for a Windows package:

```text
PocketPlay/
  PocketPlay.exe
  tools/native/
    adb.exe
    AdbWinApi.dll
    AdbWinUsbApi.dll
    SDL3.dll
    avcodec-63.dll
    avutil-61.dll
    swresample-7.dll
    libpocketplay_device.so
    pocketplay-launcher.jar
```

Obtain ADB from Android SDK Platform Tools and use SDL3/FFmpeg builds matching the committed headers and required APIs. The existing development distribution used FFmpeg 9.0.2, codec ABI 63, and media binaries from the official scrcpy 5.0 Windows bundle. PocketPlay does not invoke or package scrcpy.exe or scrcpy-server.

Include the applicable dependency licenses, provenance, and corresponding source information when publishing a binary release. Keep binary packages in GitHub Releases rather than committing them to the source tree.

## Development commands

```powershell
.\build.ps1 -Check
.\build-device.ps1 -Check
.\build.ps1 -Tests
```

`-Qa` enables development-only diagnostics. Default builds exclude them; do not publish QA builds as normal releases.
