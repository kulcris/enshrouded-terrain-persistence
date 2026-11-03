$ErrorActionPreference = 'Stop'
$cargoBin = Join-Path $PSScriptRoot 'tools/cargo/bin'
# Keep Git dependency paths below Windows MAX_PATH without global OS changes.
$env:CARGO_HOME = Join-Path $env:LOCALAPPDATA 'ETP/cargo'
$env:RUSTUP_HOME = Join-Path $PSScriptRoot 'tools/rustup'
$env:PATH = "$cargoBin;$env:CARGO_HOME/bin;$env:PATH"
$env:RUSTFLAGS = '-C linker=rust-lld -C linker-flavor=ld.lld -C link-self-contained=yes'
& "$cargoBin/cargo.exe" @args
exit $LASTEXITCODE
