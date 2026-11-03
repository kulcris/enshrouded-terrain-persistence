param([switch]$SkipBuild)
$ErrorActionPreference='Stop'
if (!$SkipBuild) {
    if (Test-Path (Join-Path $PSScriptRoot 'tools/cargo/bin/cargo.exe')) {
        & (Join-Path $PSScriptRoot 'tools-build.ps1') build --release --locked
    } else { & cargo build --release --locked }
    if ($LASTEXITCODE -ne 0) {throw 'Release build failed'}
}
$cargoText=Get-Content (Join-Path $PSScriptRoot 'Cargo.toml') -Raw
$version=[regex]::Match($cargoText,'(?m)^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"').Groups[1].Value
if (!$version) {throw 'Cannot read package version'}
$dll=Join-Path $PSScriptRoot 'target/release/terrain_persistence.dll'
if ([Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($dll)).Contains('terrain-test-command.txt')) {
    throw 'Refusing to package a build containing disposable-server test commands'
}
$dist=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'dist'))
$stage=Join-Path $dist ('.stage-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force (Join-Path $stage 'mods') | Out-Null
try {
    Copy-Item -LiteralPath $dll -Destination (Join-Path $stage 'mods/terrain_persistence.dll')
    foreach ($name in @('README.md','LICENSE','THIRD-PARTY-NOTICES.txt','config.example.toml')) {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination (Join-Path $stage $name)
    }
    $files=Get-ChildItem -LiteralPath $stage -File -Recurse | ForEach-Object {
        @{path=[IO.Path]::GetRelativePath($stage,$_.FullName).Replace('\','/');sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}
    }
    @{version=$version;experimental=$true;compatibility='signature-scanned';testedServerBuilds=@(26083076,26319703);enshwickCommit='e0c8a0bc0e9571860ca9e76ac99a8db4a66e5172';loaderIncluded=$false;files=@($files)} | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $stage 'manifest.json') -Encoding utf8
    $zip=Join-Path $dist "terrain-persistence-$version-windows.zip"
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -Force
    $hash=(Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $([IO.Path]::GetFileName($zip))" | Set-Content "$zip.sha256" -Encoding ascii
    Get-Item -LiteralPath $zip | Select-Object FullName,Length
} finally {
    $resolvedStage=[IO.Path]::GetFullPath($stage)
    if (!$resolvedStage.StartsWith($dist+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) {throw 'Unsafe stage cleanup path'}
    Remove-Item -LiteralPath $resolvedStage -Recurse -Force
}
