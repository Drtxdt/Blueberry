[CmdletBinding()]
param(
    [string]$OutputPath,
    [string]$CargoCommand = 'cargo',
    [string]$TargetTriple = 'x86_64-pc-windows-msvc',
    [switch]$IncludeDevDependencies
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $repoRoot 'THIRD-PARTY-NOTICES.txt'
}
$OutputPath = [IO.Path]::GetFullPath($OutputPath)

function Convert-ToArray {
    param([AllowNull()][object]$Value)
    if ($null -eq $Value) {
        return @()
    }
    if ($Value -is [Array]) {
        return @($Value)
    }
    return @($Value)
}

function Get-SafeText {
    param([Parameter(Mandatory = $true)][string]$Path)
    $text = [IO.File]::ReadAllText($Path)
    return $text.Replace("`r`n", "`n").Replace("`r", "`n").TrimEnd()
}

function Get-LicenseFiles {
    param(
        [Parameter(Mandatory = $true)]$Package,
        [Parameter(Mandatory = $true)][string]$CrateRoot
    )

    $paths = [Collections.Generic.List[string]]::new()
    if ($Package.ContainsKey('license_file') -and
        -not [string]::IsNullOrWhiteSpace([string]$Package.license_file)) {
        $declared = Join-Path $CrateRoot ([string]$Package.license_file)
        if (Test-Path -LiteralPath $declared -PathType Leaf) {
            $paths.Add((Get-Item -LiteralPath $declared -Force).FullName)
        }
    }
    if (Test-Path -LiteralPath $CrateRoot -PathType Container) {
        foreach ($file in (Get-ChildItem -LiteralPath $CrateRoot -File -Force | Sort-Object Name)) {
            if ($file.Name -match '^(?i:LICENSE|COPYING|NOTICE)') {
                if (-not $paths.Contains($file.FullName)) {
                    $paths.Add($file.FullName)
                }
            }
        }
    }
    return @($paths | Sort-Object)
}

function Get-NonDevPackageIds {
    param(
        [Parameter(Mandatory = $true)]$Metadata,
        [switch]$IncludeDev
    )

    $packages = @{}
    foreach ($package in $Metadata.packages) {
        $packages[[string]$package.id] = $package
    }
    if (-not $Metadata.ContainsKey('resolve') -or $null -eq $Metadata.resolve) {
        throw 'cargo metadata 没有 resolve 图；无法确认实际依赖集合'
    }
    $nodes = @{}
    foreach ($node in $Metadata.resolve.nodes) {
        $nodes[[string]$node.id] = $node
    }
    $rootId = [string]$Metadata.resolve.root
    if ([string]::IsNullOrWhiteSpace($rootId) -or -not $nodes.ContainsKey($rootId)) {
        throw 'cargo metadata 没有可遍历的 workspace root'
    }
    $visited = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $selected = [Collections.Generic.List[object]]::new()
    function Visit-Node {
        param([Parameter(Mandatory = $true)][string]$Id)
        if (-not $visited.Add($Id)) {
            return
        }
        if ($Id -ne $rootId -and $packages.ContainsKey($Id)) {
            $selected.Add($packages[$Id])
        }
        $node = $nodes[$Id]
        $package = $packages[$Id]
        $nonDevNames = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        if ($package.ContainsKey('dependencies')) {
            foreach ($declared in (Convert-ToArray $package.dependencies)) {
                $kind = if ($declared.ContainsKey('kind')) { [string]$declared.kind } else { '' }
                if ($IncludeDev.IsPresent -or $kind -ne 'dev') {
                    $dependencyName = if ($declared.ContainsKey('package') -and $declared.package) {
                        [string]$declared.package
                    } else {
                        [string]$declared.name
                    }
                    [void]$nonDevNames.Add($dependencyName)
                }
            }
        }
        foreach ($dependency in (Convert-ToArray $node.dependencies)) {
            if ($dependency -is [string]) {
                $dependencyId = [string]$dependency
                if (-not $packages.ContainsKey($dependencyId)) {
                    continue
                }
                $dependencyPackage = $packages[$dependencyId]
                $include = $IncludeDev.IsPresent -or $nonDevNames.Contains([string]$dependencyPackage.name)
            } else {
                $include = $IncludeDev.IsPresent
                if (-not $include -and $dependency.ContainsKey('dep_kinds')) {
                    $kinds = @(Convert-ToArray $dependency.dep_kinds)
                    if ($kinds.Count -eq 0) {
                        $include = $true
                    } else {
                        foreach ($kind in $kinds) {
                            if ([string]$kind.kind -ne 'dev') {
                                $include = $true
                                break
                            }
                        }
                    }
                } elseif (-not $include) {
                    # A future metadata representation may omit dep_kinds;
                    # retain the dependency rather than silently dropping it.
                    $include = $true
                }
                $dependencyId = [string]$dependency.pkg
            }
            if ($include -and $nodes.ContainsKey($dependencyId)) {
                Visit-Node -Id $dependencyId
            }
        }
    }
    Visit-Node -Id $rootId
    return @($selected | Sort-Object @{ Expression = { [string]$_.name } }, @{ Expression = { [string]$_.version } }, @{ Expression = { [string]$_.id } })
}

function Write-Utf8Atomic {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Content
    )
    $parent = Split-Path -Parent $Path
    if ($parent) {
        [IO.Directory]::CreateDirectory($parent) | Out-Null
    }
    $temporary = "$Path.new-$([Guid]::NewGuid().ToString('N'))"
    try {
        [IO.File]::WriteAllText($temporary, $Content, [Text.UTF8Encoding]::new($false))
        [IO.File]::Move($temporary, $Path, $true)
    }
    finally {
        if (Test-Path -LiteralPath $temporary) {
            Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
        }
    }
}

$metadataJson = & $CargoCommand metadata --offline --locked --format-version 1 `
    --filter-platform $TargetTriple | Out-String
if ($LASTEXITCODE -ne 0) {
    throw "cargo metadata --offline --locked 失败；没有生成许可证文件（exit $LASTEXITCODE）"
}
try {
    $metadata = $metadataJson | ConvertFrom-Json -AsHashtable
}
catch {
    throw "cargo metadata 输出不是有效 JSON: $($_.Exception.Message)"
}
if (-not ($metadata -is [Collections.IDictionary])) {
    throw 'cargo metadata 根对象无效'
}

$packages = Get-NonDevPackageIds -Metadata $metadata -IncludeDev:$IncludeDevDependencies
$missing = [Collections.Generic.List[string]]::new()
$sections = [Collections.Generic.List[string]]::new()
$sections.Add('ShellSense Beta dependency license notices')
$sections.Add('===============================================')
$sections.Add('')
$sections.Add("Generated by: cargo metadata --offline --locked --filter-platform $TargetTriple")
$sections.Add("Repository package: $($metadata.packages | Where-Object { $null -eq $_.source } | Select-Object -First 1 -ExpandProperty name)")
$sections.Add("Include dev dependencies: $($IncludeDevDependencies.IsPresent)")
$sections.Add('')
$sections.Add('Each section records the resolved package, its Cargo license expression,')
$sections.Add('the manifest path used as provenance, and the license text copied from')
$sections.Add('the crate root. No license text is synthesized from an SPDX expression.')
$sections.Add('')

foreach ($package in $packages) {
    $manifestPath = [IO.Path]::GetFullPath(([string]$package.manifest_path).Replace('/', '\'))
    $crateRoot = Split-Path -Parent $manifestPath
    $licenseFiles = @(Get-LicenseFiles -Package $package -CrateRoot $crateRoot)
    $header = "[$($package.name) $($package.version)]"
    $sections.Add($header)
    $sections.Add(('source: ' + ($(if ($package.source) { [string]$package.source } else { 'path' }))))
    # Keep the generated notice portable: the absolute Cargo registry path is
    # machine-specific and must not leak into a distributable artifact.
    $manifestOrigin = if ($package.source) {
        "registry crate root: $($package.name)-$($package.version)/Cargo.toml"
    } else {
        "local package manifest: $([IO.Path]::GetFileName($manifestPath))"
    }
    $sections.Add(('manifest: ' + $manifestOrigin))
    $sections.Add(('license: ' + ($(if ($package.license) { [string]$package.license } else { '(not declared)' }))))
    if ($licenseFiles.Count -eq 0) {
        $missing.Add("$($package.name) $($package.version): no LICENSE*/COPYING*/NOTICE* file under $crateRoot")
        $sections.Add('LICENSE TEXT MISSING: this package must be reviewed before distribution.')
    } else {
        foreach ($licenseFile in $licenseFiles) {
            try {
                $licenseText = Get-SafeText -Path $licenseFile
            }
            catch {
                $missing.Add("$($package.name) $($package.version): cannot read $licenseFile ($($_.Exception.Message))")
                $licenseText = 'LICENSE TEXT MISSING: read failed; this package must be reviewed before distribution.'
            }
            $sections.Add(('file: ' + [IO.Path]::GetFileName($licenseFile)))
            $sections.Add($licenseText)
        }
    }
    $sections.Add('')
}

if ($missing.Count -gt 0) {
    $sections.Add('MISSING LICENSE TEXT')
    $sections.Add('=====================')
    foreach ($gap in $missing) {
        $sections.Add($gap)
    }
    $missingReport = "$OutputPath.missing"
    Write-Utf8Atomic -Path $missingReport -Content (($sections -join "`n") + "`n")
    throw "缺少完整许可证原文；已写出缺口报告: $missingReport"
}

Write-Utf8Atomic -Path $OutputPath -Content (($sections -join "`n") + "`n")
Write-Host "已从 $($packages.Count) 个实际解析依赖生成 $OutputPath"
