param([Parameter(Mandatory=$true)][string]$OutputDirectory)
$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
$output=New-Item -ItemType Directory -Path $OutputDirectory -Force
$exe=Join-Path $output.FullName 'native-overlay-client-tests.exe'
& "$env:SystemRoot\Microsoft.NET\Framework64\v4.0.30319\csc.exe" /nologo /target:exe /optimize+ "/out:$exe" /reference:System.Core.dll (Join-Path $root 'shell/direct-bridge.cs') (Join-Path $root 'shell/direct-diagnostics.cs') (Join-Path $root 'shell/native-overlay.cs') (Join-Path $root 'tests/native_overlay_client.cs')
if($LASTEXITCODE) { throw 'Native overlay client test compilation failed' }
& $exe
if($LASTEXITCODE) { throw 'Native overlay client regressions failed' }
if($PSVersionTable.PSVersion.Major -ge 6) {
    $assembly=[Reflection.Assembly]::Load([IO.File]::ReadAllBytes($exe))
    if($assembly.EntryPoint.Invoke($null,@()) -ne 0) { throw 'CoreCLR native overlay client regressions failed' }
}
