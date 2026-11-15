$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
try {
    $versionLine = Get-Content Cargo.toml | Where-Object { $_ -match '^version = "[0-9]+\.[0-9]+\.[0-9]+"$' } | Select-Object -First 1
    if (-not $versionLine) { throw 'A stable application version is required.' }
    $version = ($versionLine -split '"')[1]
    cargo build --locked --release --target x86_64-pc-windows-msvc | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'Astral did not build.' }
    $targetDirectory = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
    $binary = Join-Path $targetDirectory 'x86_64-pc-windows-msvc/release/astral-native.exe'
    $output = New-Item -ItemType Directory -Force '../release'
    $archive = Join-Path $output.FullName "Astral-Native-$version-win-x64.zip"
    Compress-Archive -LiteralPath $binary -DestinationPath $archive -Force
    Write-Output $archive
} finally {
    Pop-Location
}
