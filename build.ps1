param([switch]$Check, [switch]$Tests, [switch]$Qa)
$ErrorActionPreference = 'Stop'
$projectPath = $PSScriptRoot
$workspacePath = (Resolve-Path -LiteralPath (Join-Path $projectPath '../..')).Path
$toolchainPath = Join-Path $workspacePath 'work/rust-toolchain'
if (Test-Path -LiteralPath (Join-Path $toolchainPath 'cargo/bin/cargo.exe')) {
    $env:CARGO_HOME = Join-Path $toolchainPath 'cargo'
    $env:RUSTUP_HOME = Join-Path $toolchainPath 'rustup'
    $rustPath = Join-Path $env:RUSTUP_HOME 'toolchains/stable-x86_64-pc-windows-gnu'
    $linkerPath = Join-Path $toolchainPath 'linker'
    New-Item -ItemType Directory -Path $linkerPath -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $rustPath 'lib/rustlib/x86_64-pc-windows-gnu/bin/rust-lld.exe') -Destination (Join-Path $linkerPath 'ld.lld.exe') -Force
    $llvmBinPath = Join-Path $toolchainPath 'llvm-mingw-20261006-ucrt-x86_64/bin'
    Copy-Item -LiteralPath (Join-Path $llvmBinPath 'llvm-dlltool.exe') -Destination (Join-Path $linkerPath 'dlltool.exe') -Force
    Copy-Item -LiteralPath (Join-Path $llvmBinPath 'llvm-windres.exe') -Destination (Join-Path $llvmBinPath 'windres.exe') -Force
    $env:PATH = "$linkerPath;$llvmBinPath;$env:CARGO_HOME\bin;$rustPath\bin;$env:PATH"
    $env:RUSTFLAGS = "-C linker=$linkerPath\ld.lld.exe -C linker-flavor=ld -C link-self-contained=yes"
    $env:POCKETPLAY_MINGW_LIB = Join-Path $toolchainPath 'llvm-mingw-20261006-ucrt-x86_64/x86_64-w64-mingw32/lib'
    $cargoPath = Join-Path $env:CARGO_HOME 'bin/cargo.exe'
} else {
    $cargoPath = (Get-Command cargo -ErrorAction Stop).Source
}
if ($Tests) { & $cargoPath test --manifest-path (Join-Path $projectPath 'Cargo.toml') }
elseif ($Check) { & $cargoPath check --manifest-path (Join-Path $projectPath 'Cargo.toml') }
elseif ($Qa) { & $cargoPath build --release --features qa --manifest-path (Join-Path $projectPath 'Cargo.toml') }
else { & $cargoPath build --release --manifest-path (Join-Path $projectPath 'Cargo.toml') }
if ($LASTEXITCODE -ne 0) { throw "Cargo exited with code $LASTEXITCODE." }
