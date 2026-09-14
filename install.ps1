[CmdletBinding()]
param(
    [string]$Version,
    [string]$PackagePath,
    [string]$InstallRoot = (
        Join-Path (
            [Environment]::GetFolderPath('LocalApplicationData')
        ) 'Blueberry\bin'
    ),
    [string]$ConfigRoot = (
        Join-Path (
            [Environment]::GetFolderPath('ApplicationData')
        ) 'Blueberry'
    ),
    [switch]$NoPrompt,
    [switch]$EnableStartup,
    [switch]$NoPath
)

$ErrorActionPreference = 'Stop'

# ----------------------------------------------------------------------
# Basic validation
# ----------------------------------------------------------------------

if (-not [Environment]::Is64BitOperatingSystem) {
    throw 'Blueberry requires Windows x64.'
}

if (
    $Version -and
    $Version -notmatch '^v?\d+\.\d+\.\d+(?:-[0-9A-Za-z][0-9A-Za-z.-]*)?$'
) {
    throw 'Invalid version.'
}

$InstallRoot = [IO.Path]::GetFullPath($InstallRoot)

$tempRoot = Join-Path (
    [IO.Path]::GetTempPath()
) (
    'blueberry-download-' + [Guid]::NewGuid().ToString('N')
)

[IO.Directory]::CreateDirectory($tempRoot) | Out-Null

$oldProtocol = [Net.ServicePointManager]::SecurityProtocol

try {
    # ------------------------------------------------------------------
    # TLS
    # ------------------------------------------------------------------

    [Net.ServicePointManager]::SecurityProtocol = (
        $oldProtocol -bor [Net.SecurityProtocolType]::Tls12
    )

    # ------------------------------------------------------------------
    # Download package from GitHub Releases
    # ------------------------------------------------------------------

    if (-not $PackagePath) {
        $headers = @{
            'User-Agent' = 'Blueberry-Installer'
            'Accept'     = 'application/vnd.github+json'
        }

        $api = 'https://api.github.com/repos/Drtxdt/Blueberry/releases'

        # --------------------------------------------------------------
        # Explicit version
        # --------------------------------------------------------------

        if ($Version) {
            $normalizedVersion = $Version.TrimStart('v')
            $tag = 'v' + $normalizedVersion

            try {
                $release = Invoke-RestMethod `
                    -Uri ($api + '/tags/' + $tag) `
                    -Headers $headers
            }
            catch {
                throw "Blueberry release $tag was not found."
            }

            if ($release.draft) {
                throw "Blueberry release $tag is still a draft."
            }
        }

        # --------------------------------------------------------------
        # Automatic version selection
        #
        # Priority:
        #   1. newest stable release
        #   2. newest prerelease if no stable release exists
        # --------------------------------------------------------------

        else {
            $published = @()

            for ($page = 1; $page -le 10; $page++) {
                $uri = (
                    $api +
                    '?per_page=100&page=' +
                    $page
                )

                $response = Invoke-RestMethod `
                    -Uri $uri `
                    -Headers $headers

                # Important:
                # Explicitly enumerate the returned JSON array.
                # Invoke-RestMethod may otherwise preserve the top-level
                # JSON array as a single pipeline object.
                $batch = @()

                foreach ($item in $response) {
                    $batch += $item
                }

                foreach ($item in $batch) {
                    if (
                        -not $item.draft -and
                        $item.tag_name -match '^v\d+\.\d+\.\d+(?:-[0-9A-Za-z][0-9A-Za-z.-]*)?$'
                    ) {
                        $published += $item
                    }
                }

                if ($batch.Count -lt 100) {
                    break
                }
            }

            if ($published.Count -eq 0) {
                throw 'No public Blueberry release is available yet.'
            }

            # Stable means:
            #   - GitHub prerelease flag is false
            #   - version tag contains no prerelease suffix
            #
            # Examples:
            #   v1.0.0          -> stable
            #   v1.0.0-beta.1   -> prerelease
            $stable = @(
                $published | Where-Object {
                    -not $_.prerelease -and
                    $_.tag_name -notmatch '-'
                }
            )

            if ($stable.Count -gt 0) {
                $candidates = $stable
            }
            else {
                $candidates = $published
            }

            # Sort first by semantic version core,
            # then by publication time.
            #
            # Example:
            #   v0.5.0-beta.3
            #   -> 0.5.0
            #
            # For multiple prereleases sharing the same core version,
            # published_at determines the latest one.
            $release = $candidates |
                Sort-Object `
                    @{
                        Expression = {
                            $coreVersion = (
                                $_.tag_name.TrimStart('v') -split '-'
                            )[0]

                            [version]$coreVersion
                        }
                        Descending = $true
                    },
                    @{
                        Expression = {
                            [datetime]$_.published_at
                        }
                        Descending = $true
                    } |
                Select-Object -First 1

            if (-not $release) {
                throw 'Failed to select a Blueberry release.'
            }
        }

        # ------------------------------------------------------------------
        # Validate selected release
        # ------------------------------------------------------------------

        if (-not $release.tag_name) {
            throw 'Selected release has no tag name.'
        }

        if ($release.draft) {
            throw 'Selected release is still a draft.'
        }

        Write-Host (
            'Installing Blueberry ' +
            $release.tag_name +
            $(if ($release.prerelease) { ' (prerelease)' } else { '' })
        )

        # ------------------------------------------------------------------
        # Locate release assets
        # ------------------------------------------------------------------

        $name = (
            'blueberry-' +
            $release.tag_name +
            '-windows-x64.zip'
        )

        $requiredAssets = @(
            $name,
            ($name + '.sha256')
        )

        foreach ($assetName in $requiredAssets) {
            $assets = @(
                $release.assets |
                    Where-Object {
                        $_.name -ceq $assetName
                    }
            )

            if ($assets.Count -ne 1) {
                throw "Release is missing required asset: $assetName"
            }

            $asset = $assets[0]

            $expectedUrl = (
                'https://github.com/Drtxdt/Blueberry/releases/download/' +
                $release.tag_name +
                '/' +
                $assetName
            )

            if ($asset.browser_download_url -cne $expectedUrl) {
                throw "Unexpected release asset URL for $assetName."
            }

            Write-Host (
                'Downloading ' +
                $assetName +
                '...'
            )

            Invoke-WebRequest `
                -UseBasicParsing `
                -Uri $expectedUrl `
                -OutFile (
                    Join-Path $tempRoot $assetName
                )
        }

        $PackagePath = Join-Path $tempRoot $name
    }

    # ------------------------------------------------------------------
    # Package verification
    # ------------------------------------------------------------------

    $PackagePath = [IO.Path]::GetFullPath($PackagePath)

    if (-not [IO.File]::Exists($PackagePath)) {
        throw "Package not found: $PackagePath"
    }

    $checksumPath = $PackagePath + '.sha256'

    if (-not [IO.File]::Exists($checksumPath)) {
        throw "Checksum file not found: $checksumPath"
    }

    $checksum = (
        [IO.File]::ReadAllText($checksumPath)
    ).Trim()

    $checksumPattern = (
        '^([0-9a-fA-F]{64})\s+\*?' +
        [regex]::Escape(
            [IO.Path]::GetFileName($PackagePath)
        ) +
        '$'
    )

    $checksumMatch = [regex]::Match(
        $checksum,
        $checksumPattern
    )

    if (-not $checksumMatch.Success) {
        throw 'Invalid SHA-256 checksum file format.'
    }

    $actualHash = (
        Get-FileHash `
            -LiteralPath $PackagePath `
            -Algorithm SHA256
    ).Hash

    $expectedHash = $checksumMatch.Groups[1].Value

    if ($actualHash -ine $expectedHash) {
        throw 'Package SHA-256 verification failed.'
    }

    Write-Host 'Package SHA-256 verified.'

    # ------------------------------------------------------------------
    # Extract installer helpers
    # ------------------------------------------------------------------

    Add-Type -AssemblyName System.IO.Compression.FileSystem

    $zip = [IO.Compression.ZipFile]::OpenRead(
        $PackagePath
    )

    try {
        foreach (
            $entryName in @(
                'manage-install.ps1',
                'install-common.ps1'
            )
        ) {
            $entries = @(
                $zip.Entries |
                    Where-Object {
                        $_.FullName -ceq $entryName
                    }
            )

            if (
                $entries.Count -ne 1 -or
                $entries[0].Length -gt 1048576
            ) {
                throw "Invalid installer entry: $entryName"
            }

            $destination = Join-Path (
                $tempRoot
            ) $entryName

            [IO.Compression.ZipFileExtensions]::ExtractToFile(
                $entries[0],
                $destination,
                $false
            )
        }
    }
    finally {
        $zip.Dispose()
    }

    # ------------------------------------------------------------------
    # Load common installer functions
    # ------------------------------------------------------------------

    . (
        Join-Path $tempRoot 'install-common.ps1'
    )

    # ------------------------------------------------------------------
    # Install / upgrade
    # ------------------------------------------------------------------

    $installMetadataPath = Join-Path (
        $InstallRoot
    ) 'install.json'

    $action = if (
        [IO.File]::Exists($installMetadataPath)
    ) {
        'Upgrade'
    }
    else {
        'Install'
    }

    & (
        Join-Path $tempRoot 'manage-install.ps1'
    ) `
        -Action $action `
        -PackagePath $PackagePath `
        -InstallRoot $InstallRoot

    if (-not $?) {
        throw 'Installation failed.'
    }

    # ------------------------------------------------------------------
    # Registration
    # ------------------------------------------------------------------

    $registrationPath = Join-Path (
        $InstallRoot
    ) 'registration.json'

    $registration = @{
        path_added = $false
        root       = $InstallRoot
    }

    if ([IO.File]::Exists($registrationPath)) {
        $registration = (
            Get-Content `
                -LiteralPath $registrationPath `
                -Raw `
                -Encoding UTF8
        ) | ConvertFrom-BlueberryInstallJson
    }

    # ------------------------------------------------------------------
    # PATH
    # ------------------------------------------------------------------

    if (-not $NoPath) {
        $userPath = [Environment]::GetEnvironmentVariable(
            'Path',
            'User'
        )

        $entries = @(
            $userPath -split ';' |
                Where-Object { $_ }
        )

        $alreadyInUserPath = @(
            $entries |
                Where-Object {
                    $_.TrimEnd('\') -ieq
                    $InstallRoot.TrimEnd('\')
                }
        ).Count -gt 0

        if (-not $alreadyInUserPath) {
            $registration.path_added = $true

            [IO.File]::WriteAllText(
                $registrationPath,
                ($registration | ConvertTo-Json),
                [Text.UTF8Encoding]::new($false)
            )

            [Environment]::SetEnvironmentVariable(
                'Path',
                (($entries + $InstallRoot) -join ';'),
                'User'
            )
        }

        $alreadyInProcessPath = @(
            $env:Path -split ';' |
                Where-Object {
                    $_.TrimEnd('\') -ieq
                    $InstallRoot.TrimEnd('\')
                }
        ).Count -gt 0

        if (-not $alreadyInProcessPath) {
            $env:Path = (
                $InstallRoot +
                ';' +
                $env:Path
            )
        }
    }

    [IO.File]::WriteAllText(
        $registrationPath,
        ($registration | ConvertTo-Json),
        [Text.UTF8Encoding]::new($false)
    )

    # ------------------------------------------------------------------
    # Default config
    # ------------------------------------------------------------------

    $configRoot = [IO.Path]::GetFullPath(
        $ConfigRoot
    )

    $configPath = Join-Path (
        $configRoot
    ) 'config.toml'

    if (-not [IO.File]::Exists($configPath)) {
        [IO.Directory]::CreateDirectory(
            $configRoot
        ) | Out-Null

        $exampleConfigPath = Join-Path (
            $InstallRoot
        ) 'config.example.toml'

        if (-not [IO.File]::Exists($exampleConfigPath)) {
            throw 'Installed package is missing config.example.toml.'
        }

        $config = [IO.File]::ReadAllText(
            $exampleConfigPath
        ).Replace(
            'icon_style = "nerd"',
            'icon_style = "unicode"'
        )

        [IO.File]::WriteAllText(
            $configPath,
            $config,
            [Text.UTF8Encoding]::new($false)
        )
    }

    # ------------------------------------------------------------------
    # Startup configuration
    # ------------------------------------------------------------------

    $startup = $EnableStartup.IsPresent

    $nonInteractive = @(
        [Environment]::GetCommandLineArgs() |
            Where-Object {
                $argument = [string]$_

                $argument.StartsWith('-') -and
                $argument.Length -gt 1 -and
                'NonInteractive'.StartsWith(
                    $argument.Substring(1),
                    [StringComparison]::OrdinalIgnoreCase
                )
            }
    ).Count -gt 0

    if (
        -not $startup -and
        -not $NoPrompt -and
        -not $nonInteractive -and
        [Environment]::UserInteractive -and
        -not [Console]::IsInputRedirected
    ) {
        $answer = Read-Host (
            'Start Blueberry whenever PowerShell opens? [y/N]'
        )

        $startup = (
            $answer -match '^(?i:y|yes)$'
        )
    }

    # ------------------------------------------------------------------
    # Final executable
    # ------------------------------------------------------------------

    $exe = Join-Path (
        $InstallRoot
    ) 'blueberry.exe'

    if (-not [IO.File]::Exists($exe)) {
        throw "Installation completed but executable was not found: $exe"
    }

    if ($startup) {
        & $exe startup enable

        if ($LASTEXITCODE -ne 0) {
            throw (
                'Installed; startup configuration failed. ' +
                'Run "blueberry startup enable" to retry.'
            )
        }
    }

    # ------------------------------------------------------------------
    # Done
    # ------------------------------------------------------------------

    Write-Host ''
    Write-Host "Installed: $exe"
    Write-Host 'Run: blueberry'
    Write-Host 'Startup: blueberry startup enable | disable | status'
}
finally {
    # ------------------------------------------------------------------
    # Restore TLS settings
    # ------------------------------------------------------------------

    [Net.ServicePointManager]::SecurityProtocol = $oldProtocol

    # ------------------------------------------------------------------
    # Secure temporary directory cleanup
    # ------------------------------------------------------------------

    $allowedRoot = (
        [IO.Path]::GetFullPath(
            [IO.Path]::GetTempPath()
        ).TrimEnd('\', '/') +
        [IO.Path]::DirectorySeparatorChar
    )

    $resolvedTempRoot = [IO.Path]::GetFullPath(
        $tempRoot
    )

    if (
        $resolvedTempRoot.StartsWith(
            $allowedRoot,
            [StringComparison]::OrdinalIgnoreCase
        ) -and
        [IO.Directory]::Exists($tempRoot)
    ) {
        Remove-Item `
            -LiteralPath $tempRoot `
            -Recurse `
            -Force
    }
}