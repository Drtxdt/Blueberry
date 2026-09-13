[CmdletBinding()]
param(
    [string]$Version,
    [string]$PackagePath,
    [string]$InstallRoot = (Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Blueberry\bin'),
    [string]$ConfigRoot = (Join-Path ([Environment]::GetFolderPath('ApplicationData')) 'Blueberry'),
    [switch]$NoPrompt,
    [switch]$EnableStartup,
    [switch]$NoPath
)
$ErrorActionPreference = 'Stop'
if (-not [Environment]::Is64BitOperatingSystem) { throw 'Blueberry requires Windows x64.' }
if ($Version -and $Version -notmatch '^v?\d+\.\d+\.\d+(?:-[0-9A-Za-z][0-9A-Za-z.-]*)?$') { throw 'Invalid version.' }
$InstallRoot = [IO.Path]::GetFullPath($InstallRoot)
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ('blueberry-download-' + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($tempRoot) | Out-Null
$oldProtocol = [Net.ServicePointManager]::SecurityProtocol
try {
    [Net.ServicePointManager]::SecurityProtocol = $oldProtocol -bor [Net.SecurityProtocolType]::Tls12
    if (-not $PackagePath) {
        $headers = @{ 'User-Agent' = 'Blueberry-Installer'; Accept = 'application/vnd.github+json' }
        $api = 'https://api.github.com/repos/Drtxdt/Blueberry/releases'
        if ($Version) {
            $release = Invoke-RestMethod -Uri ($api + '/tags/v' + $Version.TrimStart('v')) -Headers $headers
            if ($release.draft) { throw 'This release is a draft.' }
        } else {
            $published = @()
            for ($page = 1; $page -le 10; $page++) {
                $batch = @(Invoke-RestMethod -Uri ($api + '?per_page=100&page=' + $page) -Headers $headers)
                $published += @($batch | Where-Object { -not $_.draft -and $_.tag_name -match '^v\d+\.\d+\.\d+(?:-[0-9A-Za-z][0-9A-Za-z.-]*)?$' })
                if ($batch.Count -lt 100) { break }
            }
            $stable = @($published | Where-Object { -not $_.prerelease -and $_.tag_name -notmatch '-' })
            $candidates = if ($stable.Count) { $stable } else { $published }
            $release = $candidates | Sort-Object @{Expression = { [version](($_.tag_name.TrimStart('v') -split '-')[0]) }; Descending=$true}, @{Expression='published_at';Descending=$true} | Select-Object -First 1
            if (-not $release) { throw 'No public Blueberry release is available yet. Publish the Release draft first.' }
        }
        Write-Host ('Installing Blueberry ' + $release.tag_name)
        $name = 'blueberry-' + $release.tag_name + '-windows-x64.zip'
        foreach ($assetName in @($name, ($name + '.sha256'))) {
            $asset = @($release.assets | Where-Object { $_.name -ceq $assetName })
            if ($asset.Count -ne 1) { throw "Release is missing $assetName" }
            $expectedUrl = 'https://github.com/Drtxdt/Blueberry/releases/download/' + $release.tag_name + '/' + $assetName
            if ($asset[0].browser_download_url -cne $expectedUrl) { throw 'Unexpected release asset URL.' }
            Invoke-WebRequest -UseBasicParsing -Uri $expectedUrl -OutFile (Join-Path $tempRoot $assetName)
        }
        $PackagePath = Join-Path $tempRoot $name
    }
    $PackagePath = [IO.Path]::GetFullPath($PackagePath)
    $checksum = ([IO.File]::ReadAllText($PackagePath + '.sha256')).Trim()
    $checksumMatch = [regex]::Match($checksum, '^([0-9a-fA-F]{64})\s+\*?' + [regex]::Escape([IO.Path]::GetFileName($PackagePath)) + '$')
    if (-not $checksumMatch.Success -or (Get-FileHash -LiteralPath $PackagePath -Algorithm SHA256).Hash -ine $checksumMatch.Groups[1].Value) { throw 'Package SHA-256 verification failed.' }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($PackagePath)
    try {
        foreach ($name in @('manage-install.ps1','install-common.ps1')) {
            $entries = @($zip.Entries | Where-Object { $_.FullName -ceq $name })
            if ($entries.Count -ne 1 -or $entries[0].Length -gt 1048576) { throw "Invalid installer entry: $name" }
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entries[0], (Join-Path $tempRoot $name), $false)
        }
    } finally { $zip.Dispose() }
    . (Join-Path $tempRoot 'install-common.ps1')
    $action = if ([IO.File]::Exists((Join-Path $InstallRoot 'install.json'))) { 'Upgrade' } else { 'Install' }
    & (Join-Path $tempRoot 'manage-install.ps1') -Action $action -PackagePath $PackagePath -InstallRoot $InstallRoot
    if (-not $?) { throw 'Installation failed.' }
    $registrationPath = Join-Path $InstallRoot 'registration.json'
    $registration = @{ path_added = $false; root = $InstallRoot }
    if ([IO.File]::Exists($registrationPath)) { $registration = Get-Content -LiteralPath $registrationPath -Raw -Encoding UTF8 | ConvertFrom-BlueberryInstallJson }
    if (-not $NoPath) {
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        $entries = @($userPath -split ';' | Where-Object { $_ })
        if (-not @($entries | Where-Object { $_.TrimEnd('\') -ieq $InstallRoot.TrimEnd('\') }).Count) {
            $registration.path_added = $true
            [IO.File]::WriteAllText($registrationPath, ($registration | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
            [Environment]::SetEnvironmentVariable('Path', (($entries + $InstallRoot) -join ';'), 'User')
        }
        if (-not @($env:Path -split ';' | Where-Object { $_.TrimEnd('\') -ieq $InstallRoot.TrimEnd('\') }).Count) { $env:Path = $InstallRoot + ';' + $env:Path }
    }
    [IO.File]::WriteAllText($registrationPath, ($registration | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    $configRoot = [IO.Path]::GetFullPath($ConfigRoot)
    $configPath = Join-Path $configRoot 'config.toml'
    if (-not [IO.File]::Exists($configPath)) {
        [IO.Directory]::CreateDirectory($configRoot) | Out-Null
        $config = [IO.File]::ReadAllText((Join-Path $InstallRoot 'config.example.toml')).Replace('icon_style = "nerd"', 'icon_style = "unicode"')
        [IO.File]::WriteAllText($configPath, $config, [Text.UTF8Encoding]::new($false))
    }
    $startup = $EnableStartup.IsPresent
    $nonInteractive = @([Environment]::GetCommandLineArgs() | Where-Object {
        $argument = [string]$_
        $argument.StartsWith('-') -and $argument.Length -gt 1 -and 'NonInteractive'.StartsWith($argument.Substring(1), [StringComparison]::OrdinalIgnoreCase)
    }).Count -gt 0
    if (-not $startup -and -not $NoPrompt -and -not $nonInteractive -and [Environment]::UserInteractive -and -not [Console]::IsInputRedirected) {
        $startup = (Read-Host 'Start Blueberry whenever PowerShell opens? [y/N]') -match '^(?i:y|yes)$'
    }
    $exe = Join-Path $InstallRoot 'blueberry.exe'
    if ($startup) { & $exe startup enable; if ($LASTEXITCODE -ne 0) { throw 'Installed; startup configuration failed. Run blueberry startup enable to retry.' } }
    Write-Host "Installed: $exe"
    Write-Host 'Run: blueberry'
    Write-Host 'Startup: blueberry startup enable | disable | status'
} finally {
    [Net.ServicePointManager]::SecurityProtocol = $oldProtocol
    # This directory is a fixed, unique child of the OS temporary directory.
    $allowedRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar
    if ([IO.Path]::GetFullPath($tempRoot).StartsWith($allowedRoot, [StringComparison]::OrdinalIgnoreCase) -and [IO.Directory]::Exists($tempRoot)) { Remove-Item -LiteralPath $tempRoot -Recurse -Force }
}
