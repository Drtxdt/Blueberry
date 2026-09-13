$ErrorActionPreference = 'Stop'
$root = Join-Path ([IO.Path]::GetTempPath()) ('blueberry-startup-test-' + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($root) | Out-Null
$startup = Join-Path $PSScriptRoot '../scripts/startup.ps1'
try {
    foreach ($encoding in @([Text.UTF8Encoding]::new($true), [Text.UTF8Encoding]::new($false), [Text.Encoding]::Unicode)) {
        $path = Join-Path $root ('中文-' + [Guid]::NewGuid().ToString('N') + '.ps1')
        $original = "# 用户已有内容`r`nWrite-Output 'PROFILE_OK'"
        [IO.File]::WriteAllText($path, $original, $encoding)
        $before = [Convert]::ToBase64String([IO.File]::ReadAllBytes($path))
        $args = @{ ProfilePath=$path; Executable=(Join-Path $root 'missing blueberry.exe') }
        & $startup -Action enable @args | Out-Null
        $once = [Convert]::ToBase64String([IO.File]::ReadAllBytes($path))
        & $startup -Action enable @args | Out-Null
        if ($once -cne [Convert]::ToBase64String([IO.File]::ReadAllBytes($path))) { throw 'Enable is not idempotent' }
        if (-not (& $startup -Action status @args).enabled) { throw 'Status did not report enabled' }
        & $startup -Action disable @args | Out-Null
        if ($before -cne [Convert]::ToBase64String([IO.File]::ReadAllBytes($path))) { throw 'Original profile bytes changed' }
        & $startup -Action disable @args | Out-Null
        if ((& $startup -Action status @args).enabled) { throw 'Status did not report disabled' }
    }
    foreach ($original in @("using namespace System`r`nparam()`r`nWrite-Output 'ok'", 'using namespace System')) {
        $path = Join-Path $root 'headers.ps1'
        [IO.File]::WriteAllText($path, $original, [Text.UTF8Encoding]::new($true))
        $args = @{ ProfilePath=$path; Executable=(Join-Path $root 'missing.exe') }
        & $startup -Action enable @args | Out-Null
        $tokens = $null; $errors = $null
        [Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errors) | Out-Null
        if ($errors.Count) { throw "Startup hook broke a using / param header: $errors" }
        $once = [IO.File]::ReadAllText($path)
        & $startup -Action enable @args | Out-Null
        if ($once -cne [IO.File]::ReadAllText($path)) { throw 'Header hook is not idempotent' }
        & $startup -Action disable @args | Out-Null
        if ($original -cne [IO.File]::ReadAllText($path)) { throw 'Header profile was changed' }
    }
    $newPath = Join-Path $root 'new-profile.ps1'
    & $startup -Action enable -ProfilePath $newPath -Executable (Join-Path $root 'missing.exe') | Out-Null
    $env:BLUEBERRY_ACTIVE = '1'
    . $newPath
    Remove-Item Env:BLUEBERRY_ACTIVE
    Write-Output 'Startup profile tests passed.'
} finally {
    $resolved = [IO.Path]::GetFullPath($root)
    $expected = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar
    if ($resolved.StartsWith($expected, [StringComparison]::OrdinalIgnoreCase)) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
