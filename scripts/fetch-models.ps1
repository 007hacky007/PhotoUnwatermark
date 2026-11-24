# Download the models-v2 release files into $env:PUW_MODELS_DIR and verify their sha256.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$base = 'https://github.com/007hacky007/PhotoUnwatermark/releases/download/models-v2'
$dir = $env:PUW_MODELS_DIR
if (-not $dir) { throw 'set PUW_MODELS_DIR to the destination folder' }

New-Item -ItemType Directory -Force -Path $dir | Out-Null
$manifestPath = Join-Path $dir 'manifest.json'
Invoke-WebRequest -Uri "$base/manifest.json" -OutFile $manifestPath
$manifest = Get-Content -Raw $manifestPath | ConvertFrom-Json
if (-not $manifest.files) { throw 'no files listed in manifest.json' }

$failed = $false
foreach ($entry in $manifest.files) {
    $dest = Join-Path $dir $entry.name
    if ((Test-Path $dest) -and ((Get-FileHash -Algorithm SHA256 $dest).Hash -eq $entry.sha256)) {
        Write-Host "$($entry.name): present, OK"
        continue
    }
    $part = "$dest.part"
    Invoke-WebRequest -Uri "$base/$($entry.name)" -OutFile $part
    $got = (Get-FileHash -Algorithm SHA256 $part).Hash.ToLower()
    if ($got -eq $entry.sha256) {
        Move-Item -Force $part $dest
        Write-Host "$($entry.name): downloaded, OK"
    } else {
        Remove-Item -Force $part
        [Console]::Error.WriteLine("$($entry.name): sha256 mismatch (want $($entry.sha256), got $got)")
        $failed = $true
    }
}
if ($failed) { exit 1 }
