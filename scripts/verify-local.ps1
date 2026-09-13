[CmdletBinding()]
param([string]$Shell = 'pwsh.exe', [switch]$SkipCore)
$ErrorActionPreference = 'Stop'
$testEnvironment = @{}
foreach ($name in @('BLUEBERRY_TEST_SHELL','BLUEBERRY_NO_HISTORY','BLUEBERRY_TEST_TRANSPORT')) {
    $testEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
try {
    $env:BLUEBERRY_TEST_SHELL = (Get-Command $Shell -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
    $env:BLUEBERRY_NO_HISTORY = '1'
    & $env:BLUEBERRY_TEST_SHELL -NoProfile -NonInteractive -Command '$PSVersionTable.PSVersion.ToString()'
    if ($LASTEXITCODE -ne 0) { throw 'PowerShell unavailable' }
    if (-not $SkipCore) {
        cargo fmt --all -- --check
        if ($LASTEXITCODE -ne 0) { throw 'Formatting failed' }
        cargo clippy --all-targets --locked -- -D warnings
        if ($LASTEXITCODE -ne 0) { throw 'Clippy failed' }
        cargo test --locked --lib --test cli --test engine --test providers --test specs --test knowledge -- --test-threads=1
        if ($LASTEXITCODE -ne 0) { throw 'Core tests failed' }
    }
    foreach ($script in @('adapter.tests.ps1','startup.tests.ps1')) {
        $path = Join-Path $PSScriptRoot ('../tests/' + $script)
        & $env:BLUEBERRY_TEST_SHELL -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $path
        if ($LASTEXITCODE -ne 0) { throw "$script failed" }
    }
    foreach ($transport in @('osc','pipe')) {
        $env:BLUEBERRY_TEST_TRANSPORT = $transport
        cargo test --locked --test host --test startup_host --test terminal_beta --test terminal_modes -- --test-threads=1
        if ($LASTEXITCODE -ne 0) { throw "$transport terminal regressions failed" }
    }
    Write-Host 'Automated validation passed. See docs/installation.md for the Windows Terminal visual checklist.'
} finally {
    foreach ($name in $testEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $testEnvironment[$name], 'Process')
    }
}
