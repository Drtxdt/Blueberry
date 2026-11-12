[CmdletBinding()]
param(
    [string]$OutputDirectory='artifacts/review-hardening/matrix',
    [switch]$ContinueOnFailure,
    [ValidateSet('ps51-200','ps51-245','ps7-245')][string[]]$Cases=@('ps51-200','ps51-245','ps7-245'),
    [ValidateSet('direct','pipe','osc')][string[]]$Lanes=@('direct','pipe','osc')
)
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath($OutputDirectory)
$null=New-Item -ItemType Directory -Path $root -Force
$names=@('BLUEBERRY_TEST_SHELL','BLUEBERRY_TEST_PSREADLINE_MODULE','BLUEBERRY_TEST_TRANSPORT','BLUEBERRY_NO_HISTORY')
$previous=@{}
foreach($name in $names) { $previous[$name]=[Environment]::GetEnvironmentVariable($name) }
$results=@()
try {
    $env:BLUEBERRY_NO_HISTORY='1'
    foreach($case in @(
        @{ name='ps51-200'; shell="$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe"; editor='2.0.0' },
        @{ name='ps51-245'; shell="$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe"; editor='2.4.5' },
        @{ name='ps7-245'; shell=(Get-Command pwsh.exe).Source; editor='2.4.5' }
    )) {
        if($case.name -notin $Cases) { continue }
        $env:BLUEBERRY_TEST_SHELL=$case.shell
        $moduleRoot="target/review-original-editor/$($case.editor)"
        python scripts/prepare-original-editor.py --version $case.editor --root $moduleRoot
        if($LASTEXITCODE) { throw "Pinned editor preparation failed" }
        $env:BLUEBERRY_TEST_PSREADLINE_MODULE=(Resolve-Path "$moduleRoot/PSReadLine/$($case.editor)/PSReadLine.psd1").Path
        foreach($lane in @('direct','pipe','osc')) {
            if($lane -notin $Lanes) { continue }
            $log=Join-Path $root ($case.name+'-'+$lane+'.log')
            $env:BLUEBERRY_TEST_TRANSPORT=if($lane -eq 'direct') { 'pipe' } else { $lane }
            if($lane -eq 'direct') {
                & cargo test --locked --test direct_terminal --test input_protocol --test startup_host --test overlay_resize --quiet -- --test-threads=1 *> $log
            } else {
                & cargo test --locked --test host --test terminal_modes --test terminal_beta --quiet -- --test-threads=1 *> $log
            }
            $code=$LASTEXITCODE
            $build=(& ./target/debug/blueberry.exe --config config.example.toml doctor --json | ConvertFrom-Json).build
            if($LASTEXITCODE) { throw 'Cannot read tested product build identity' }
            $results+=@{ case=$case.name; lane=$lane; exit_code=$code; passed=($code -eq 0); log=$log;
                source_commit=$build.commit; source_dirty=$build.dirty; working_tree_head=(& git rev-parse HEAD); product_sha256=(Get-FileHash target/debug/blueberry.exe).Hash;
                shell_sha256=(Get-FileHash $case.shell).Hash; module_sha256=(Get-FileHash $env:BLUEBERRY_TEST_PSREADLINE_MODULE).Hash }
            ConvertTo-Json -InputObject @($results) -Depth 5 | Set-Content -LiteralPath (Join-Path $root 'results.json') -Encoding utf8
            if($code -ne 0 -and -not $ContinueOnFailure) { throw "Matrix failure: $($case.name)/$lane; evidence: $log" }
        }
    }
    if(@($results | Where-Object { -not $_.passed }).Count) { throw "Matrix completed with failures; see results.json" }
} finally {
    foreach($name in $names) { [Environment]::SetEnvironmentVariable($name,$previous[$name],'Process') }
}
