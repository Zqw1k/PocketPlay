param([switch]$Check,[switch]$Qa)
$ErrorActionPreference = 'Stop'
$projectPath = $PSScriptRoot
$workspacePath = (Resolve-Path -LiteralPath (Join-Path $projectPath '../..')).Path
$sdkPath = Join-Path $workspacePath 'work/native-sdk'
$ndkPath = Join-Path $sdkPath 'android-ndk-r28c/toolchains/llvm/prebuilt/windows-x86_64'
$env:CARGO_HOME = Join-Path $workspacePath 'work/rust-toolchain/cargo'
$env:RUSTUP_HOME = Join-Path $workspacePath 'work/rust-toolchain/rustup'
$env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = Join-Path $ndkPath 'bin/aarch64-linux-android29-clang.cmd'
Remove-Item Env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER -ErrorAction SilentlyContinue
Remove-Item Env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS -ErrorAction SilentlyContinue
$env:PATH = (($env:PATH -split ';') | Where-Object { $_ -notmatch 'llvm-mingw|rust-toolchain[\\/]linker' }) -join ';'
$env:RUSTFLAGS = ''
$cargoPath = Join-Path $env:CARGO_HOME 'bin/cargo.exe'
$manifest = Join-Path $projectPath 'android/native/Cargo.toml'
if ($Check) {
    & $cargoPath check --target aarch64-linux-android --manifest-path $manifest
    if ($LASTEXITCODE -ne 0) { throw 'Device check failed.' }
    return
}
$featureArgs=@();if ($Qa) {$featureArgs=@('--features','qa')}
& $cargoPath build --release --target aarch64-linux-android --manifest-path $manifest @featureArgs
if ($LASTEXITCODE -ne 0) { throw 'Device compilation failed.' }
$jdkBin = 'C:/Program Files/Java/jdk-26.0.2.1/bin'
if ($env:JAVA_HOME) { $jdkBin = Join-Path $env:JAVA_HOME 'bin' }
$classPath = Join-Path $projectPath 'android/classes-native'
$dexPath = Join-Path $projectPath 'android/dex-native'
New-Item -ItemType Directory -Path $classPath,$dexPath -Force | Out-Null
& (Join-Path $jdkBin 'javac.exe') --release 11 -cp (Join-Path $sdkPath 'android-35/android.jar') -d $classPath (Join-Path $projectPath 'android/src/com/pocketplay/server/Main.java')
if ($LASTEXITCODE -ne 0) { throw 'Launcher compilation failed.' }
$classJar = Join-Path $projectPath 'android/launcher.jar'
& (Join-Path $jdkBin 'jar.exe') cf $classJar -C $classPath .
if ($LASTEXITCODE -ne 0) { throw 'Launcher archive failed.' }
& (Join-Path $jdkBin 'java.exe') -cp (Join-Path $sdkPath 'android-15/lib/d8.jar') com.android.tools.r8.D8 --min-api 29 --lib (Join-Path $sdkPath 'android-35/android.jar') --output $dexPath $classJar
if ($LASTEXITCODE -ne 0) { throw 'Launcher dex compilation failed.' }
$toolsPath = Join-Path $projectPath 'tools/native'
& (Join-Path $jdkBin 'jar.exe') cf (Join-Path $toolsPath 'pocketplay-launcher.jar') -C $dexPath classes.dex
if ($LASTEXITCODE -ne 0) { throw 'Dex archive failed.' }
Copy-Item -LiteralPath (Join-Path $projectPath 'android/native/target/aarch64-linux-android/release/libpocketplay_device.so') -Destination $toolsPath -Force
