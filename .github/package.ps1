$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
$cargoText=Get-Content (Join-Path $root 'Cargo.toml') -Raw
$version=[regex]::Match($cargoText,'(?m)^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"').Groups[1].Value
if (!$version) {throw 'Cannot read package version'}
$dll=Join-Path $root 'target/release/terrain_persistence.dll'
$loader=Join-Path $root 'tools/enshwick-alpha13'
$loaderDll=Join-Path $loader 'mods/enshwick/enshwick.dll'
$loaderPower=Join-Path $loader 'powrprof.dll'
$loaderLicense=Join-Path $loader 'LICENSE'
foreach ($required in @($loaderDll,$loaderPower,$loaderLicense)) {
    if (!(Test-Path -LiteralPath $required -PathType Leaf)) {throw "Required Enshwick alpha 13 runtime file is missing: $required"}
}
$dist=[IO.Path]::GetFullPath((Join-Path $root 'dist'))
$stage=Join-Path $dist ('.stage-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force (Join-Path $stage 'mods') | Out-Null
New-Item -ItemType Directory -Force (Join-Path $stage 'mods/terrain-persistence') | Out-Null
New-Item -ItemType Directory -Force (Join-Path $stage 'mods/enshwick') | Out-Null
try {
    Copy-Item -LiteralPath $dll -Destination (Join-Path $stage 'mods/terrain-persistence/terrain_persistence.dll')
    Copy-Item -LiteralPath $loaderDll -Destination (Join-Path $stage 'mods/enshwick/enshwick.dll')
    Copy-Item -LiteralPath $loaderPower -Destination (Join-Path $stage 'powrprof.dll')
    Copy-Item -LiteralPath $loaderLicense -Destination (Join-Path $stage 'ENSHWICK-LICENSE')
    foreach ($name in @('README.md','LICENSE','THIRD-PARTY-NOTICES.txt')) {
        Copy-Item -LiteralPath (Join-Path $root $name) -Destination (Join-Path $stage $name)
    }
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
