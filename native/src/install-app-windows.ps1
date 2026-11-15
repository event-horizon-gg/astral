$ErrorActionPreference = 'Stop'
$target = $env:ASTRAL_UPDATE_TARGET
$stage = $env:ASTRAL_UPDATE_STAGE
$ack = $env:ASTRAL_UPDATE_ACK
$token = $env:ASTRAL_UPDATE_TOKEN
$parentId = [int]$env:ASTRAL_UPDATE_PARENT
$previous = Join-Path $stage 'previous.exe'
$replacement = Join-Path $stage 'astral-native.exe'
$failed = Join-Path $stage 'failed.exe'
$log = Join-Path $stage 'install.log'

function Start-Astral {
    param([string[]]$AppArgs)
    $launch = New-Object System.Diagnostics.ProcessStartInfo
    $launch.FileName = $target
    $launch.WorkingDirectory = [IO.Path]::GetDirectoryName($target)
    $launch.UseShellExecute = $false
    $launch.Arguments = $AppArgs -join ' '
    return [System.Diagnostics.Process]::Start($launch)
}

try {
    try {
        $oldProcess = [Diagnostics.Process]::GetProcessById($parentId)
    } catch [ArgumentException] {
        $oldProcess = $null
    }
    if ($null -ne $oldProcess) {
        try {
            if (-not $oldProcess.WaitForExit(60000)) {
                throw 'Astral did not exit in time.'
            }
        } finally {
            $oldProcess.Dispose()
        }
    }

    $recovery = 'game-check-failed'
    try {
        $games = @(
            foreach ($name in @('Wow', 'Wow-64', 'WowClassic', 'WowF', 'WowForever')) {
                [Diagnostics.Process]::GetProcessesByName($name)
            }
        )
        if ($games.Count -gt 0) {
            $recovery = 'wow-running'
            throw 'Close World of Warcraft to update Astral.'
        }
        $recovery = 'replacement-failed'
        if (Test-Path -LiteralPath $ack) { throw 'An update acknowledgement already exists.' }
        [IO.File]::Replace($replacement, $target, $previous)
    } catch {
        if ([IO.File]::Exists($target)) {
            Start-Astral -AppArgs @('--astral-update-recovery', $recovery) | Out-Null
        }
        throw
    }

    $child = $null
    try {
        $child = Start-Astral -AppArgs @('--astral-update-ack', $token)
        $watch = [Diagnostics.Stopwatch]::StartNew()
        while ($watch.Elapsed.TotalSeconds -lt 60) {
            if ([IO.File]::Exists($ack)) { break }
            if ($child.WaitForExit(250)) { break }
        }
    } catch {
        Add-Content -LiteralPath $log -Value $_.Exception.Message
    }

    if ([IO.File]::Exists($ack)) {
        [IO.File]::Delete($ack)
        Remove-Item -LiteralPath $stage -Recurse -Force
        exit 0
    }

    if ($null -ne $child -and -not $child.HasExited) {
        try {
            $child.Kill()
        } catch [InvalidOperationException] {
            if (-not $child.HasExited) { throw }
        }
        if (-not $child.WaitForExit(15000)) {
            throw 'The new app did not exit. The previous executable is still available in the update folder.'
        }
    }
    if ($null -ne $child) { $child.Dispose() }
    [IO.File]::Replace($previous, $target, $failed)
    try {
        [IO.File]::Delete($ack)
    } catch {
        Add-Content -LiteralPath $log -Value $_.Exception.Message
    }
    Add-Content -LiteralPath $log -Value 'The update did not start. Restored the previous version.'
    Start-Astral -AppArgs @('--astral-update-recovery', 'rolled-back') | Out-Null
} catch {
    Add-Content -LiteralPath $log -Value $_.Exception.Message
    exit 1
}
