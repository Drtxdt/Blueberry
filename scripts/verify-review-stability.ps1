[CmdletBinding()]
param([ValidateRange(1,15)][int]$RepeatsPerTrace = 15, [string]$OutputDirectory = 'artifacts/review-hardening/stability')
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath($OutputDirectory)
$null = New-Item -ItemType Directory -Path $root -Force
$names = @('BLUEBERRY_TEST_SHELL','BLUEBERRY_TEST_PSREADLINE_MODULE','BLUEBERRY_TEST_TRANSPORT','BLUEBERRY_TEST_TRACE_DISABLED','BLUEBERRY_NO_HISTORY','RUNNER_TEMP','BLUEBERRY_TEST_HOST_TRACE')
$previous = @{}
foreach ($name in $names) { $previous[$name] = [Environment]::GetEnvironmentVariable($name) }
$results = @()
try {
    $env:BLUEBERRY_NO_HISTORY = '1'
    python scripts/prepare-original-editor.py --version 2.4.5 --root target/review-stability-editor
    if ($LASTEXITCODE) { throw 'Pinned editor preparation failed' }
    $env:BLUEBERRY_TEST_PSREADLINE_MODULE = (Resolve-Path 'target/review-stability-editor/PSReadLine/2.4.5/PSReadLine.psd1').Path
    foreach ($case in @(
        @{ name='ps7-osc-resize'; shell=(Get-Command pwsh.exe).Source; target='host'; test='host_conpty_menu_accepts_options_restores_screen_and_keeps_control_keys_out'; transport='osc' },
        @{ name='ps51-navigation'; shell="$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe"; target='direct_terminal'; test='direct_bottom_menu_reserves_space_and_survives_small_viewport_navigation'; transport='pipe' }
    )) {
        $env:BLUEBERRY_TEST_SHELL = $case.shell
        $env:BLUEBERRY_TEST_TRANSPORT = $case.transport
        foreach ($trace in @('0','1')) {
            $env:BLUEBERRY_TEST_TRACE_DISABLED = $trace
            $env:BLUEBERRY_TEST_HOST_TRACE = if ($trace -eq '0') { '1' } else { $null }
            $log = Join-Path $root ($case.name+'-trace-disabled-'+$trace+'.log')
            $passed=$false
            try {
                & "$PSScriptRoot/repeat-rust-test.ps1" -Target $case.target -Test $case.test -Count $RepeatsPerTrace *> $log
                $passed=$true
            } finally {
                $results += @{ case=$case.name; trace_disabled=$trace; repetitions=$RepeatsPerTrace; passed=$passed; log=$log }
                ConvertTo-Json -InputObject @($results) -Depth 5 | Set-Content -LiteralPath (Join-Path $root 'results.json') -Encoding utf8
            }
        }
    }
} finally {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name,$previous[$name],'Process') }
}
