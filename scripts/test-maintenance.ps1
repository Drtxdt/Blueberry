[CmdletBinding()]
param([Parameter(Mandatory)][string]$Executable, [Parameter(Mandatory)][string]$Package)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
# Hosted CI itself can own a non-breakaway Job. Give this isolated fixture
# the same innermost breakaway boundary as a Blueberry session; helpers leave
# our test Job while remaining subject to the runner's outer cleanup Job.
# The blocked-Job case below adds a new non-breakaway boundary and must still
# reject helper launch and retain its durable queue.
Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
public static class BlueberryMaintenanceTestJob {
    [StructLayout(LayoutKind.Sequential)] struct BasicLimits {
        public long ProcessTime, JobTime;
        public uint Flags;
        public UIntPtr MinWorkingSet, MaxWorkingSet;
        public uint ActiveProcesses;
        public UIntPtr Affinity;
        public uint Priority, Scheduling;
    }
    [StructLayout(LayoutKind.Sequential)] struct IoCounters {
        public ulong ReadOperations, WriteOperations, OtherOperations, ReadBytes, WriteBytes, OtherBytes;
    }
    [StructLayout(LayoutKind.Sequential)] struct ExtendedLimits {
        public BasicLimits Basic;
        public IoCounters Io;
        public UIntPtr ProcessMemory, JobMemory, PeakProcessMemory, PeakJobMemory;
    }
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr CreateJobObject(IntPtr attributes,string name);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool SetInformationJobObject(IntPtr job,int type,ref ExtendedLimits information,uint length);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool AssignProcessToJobObject(IntPtr job,IntPtr process);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    public static void Attach() {
        IntPtr job=CreateJobObject(IntPtr.Zero,null);
        if(job==IntPtr.Zero)throw new System.ComponentModel.Win32Exception();
        try {
            var limits=new ExtendedLimits();
            limits.Basic.Flags=0x800; // JOB_OBJECT_LIMIT_BREAKAWAY_OK, no kill-on-close.
            if(!SetInformationJobObject(job,9,ref limits,(uint)Marshal.SizeOf(limits)) ||
                !AssignProcessToJobObject(job,Process.GetCurrentProcess().Handle))
                throw new System.ComponentModel.Win32Exception();
        } finally { CloseHandle(job); }
    }
}
'@
[BlueberryMaintenanceTestJob]::Attach()
. (Join-Path $PSScriptRoot 'install-common.ps1')
$canonical='{"records":[{"bytes":3,"path":"a"}],"touched":["README.md"]}'
if ((ConvertTo-BlueberryCanonicalJson ($canonical | ConvertFrom-BlueberryInstallJson)) -cne $canonical) { throw 'Nested transaction JSON did not round-trip' }
$Executable=(Resolve-Path -LiteralPath $Executable).Path
$Package=(Resolve-Path -LiteralPath $Package).Path
$scratch=Join-Path (Split-Path $PSScriptRoot) ('target/maintenance-tests-'+[Guid]::NewGuid().ToString('N'))
$root=Join-Path $scratch 'install'
[IO.Directory]::CreateDirectory($scratch)|Out-Null
$manager=Join-Path $PSScriptRoot 'manage-install.ps1'
$payload=Join-Path $scratch 'payload'
Expand-Archive -LiteralPath $Package -DestinationPath $payload
$notes=Join-Path $scratch 'previous-notes.md'
[IO.File]::WriteAllText($notes,'Previous installation fixture')
$version=(Get-Content (Join-Path $payload 'release.json') -Raw|ConvertFrom-Json).version
$previousOutput=Join-Path $scratch 'previous-package'
& (Join-Path $PSScriptRoot 'release.ps1') -ExePath $Executable -Version $version -OutputDirectory $previousOutput -ReleaseNotesPath $notes -LicenseNoticesPath (Join-Path $payload 'THIRD-PARTY-NOTICES.txt') | Out-Null
$previousPackage=(Get-ChildItem -LiteralPath $previousOutput -Filter '*.zip').FullName
& $manager -Action Install -InstallRoot $root -PackagePath $previousPackage
$exe=Join-Path $root 'blueberry.exe'
if ((Get-FileHash $exe).Hash -ne (Get-FileHash $Executable).Hash) { throw 'Test package must contain the exact executable' }
$hash=[Security.Cryptography.SHA256]::Create()
try { $id=([BitConverter]::ToString($hash.ComputeHash([Text.Encoding]::UTF8.GetBytes($root.ToLowerInvariant())))).Replace('-','').ToLowerInvariant().Substring(0,16) } finally { $hash.Dispose() }
$stateRoot=Join-Path $scratch ('.blueberry-maintenance-'+$id)
$statePath=Join-Path $stateRoot 'operation.json'
function Read-State { Read-BlueberrySharedText $statePath | ConvertFrom-Json }
function Wait-State([string[]]$Expected) {
    $deadline=[DateTime]::UtcNow.AddSeconds(300)
    while([DateTime]::UtcNow -lt $deadline) {
        if(Test-Path -LiteralPath $statePath) {
            $state=Read-State
            if($state.status -in $Expected){return $state}
            if($state.status -in @('failed','recovery_required')){throw ($state|ConvertTo-Json -Compress)}
        }
        Start-Sleep -Milliseconds 100
    }
    throw ('Maintenance timed out: '+(Get-Content (Join-Path $stateRoot 'worker.log') -Raw -ErrorAction SilentlyContinue))
}
function Invoke-Cli([string[]]$Arguments) {
    & $exe @Arguments
    if($LASTEXITCODE -ne 0){throw "CLI failed: $Arguments"}
}
$helper=Join-Path $scratch 'hold.cs'
[IO.File]::WriteAllText($helper,@'
using System;
using System.Diagnostics;
using System.IO;
using System.Threading;
using System.Runtime.InteropServices;
class Hold {
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode)] static extern IntPtr CreateJobObject(IntPtr attributes,string name);
    [DllImport("kernel32.dll")] static extern bool AssignProcessToJobObject(IntPtr job,IntPtr process);
    static int Main() {
        string exe=Environment.GetEnvironmentVariable("BB_TEST_EXE");
        string operation=Environment.GetEnvironmentVariable("BB_TEST_OPERATION");
        bool blocked=Environment.GetEnvironmentVariable("BB_TEST_BLOCK_JOB")=="1";
        if(blocked) {
            // An additional enclosing Job deliberately forbids breakaway.
            IntPtr job=CreateJobObject(IntPtr.Zero,null);
            if(job==IntPtr.Zero || !AssignProcessToJobObject(job,Process.GetCurrentProcess().Handle))return 90;
        }
        if(operation=="hold") {
            File.WriteAllText(Environment.GetEnvironmentVariable("BB_TEST_READY"),"0\n");
            using(var wait=new EventWaitHandle(false,EventResetMode.ManualReset,Environment.GetEnvironmentVariable("BB_TEST_EVENT"))) {wait.WaitOne();}
            return 0;
        }
        string arguments=operation=="upgrade" ? "upgrade --package \""+Environment.GetEnvironmentVariable("BB_TEST_PACKAGE")+"\"" : operation;
        var info=new ProcessStartInfo(exe,arguments) {UseShellExecute=false,CreateNoWindow=true,RedirectStandardOutput=true,RedirectStandardError=true,StandardOutputEncoding=System.Text.Encoding.UTF8,StandardErrorEncoding=System.Text.Encoding.UTF8};
        using(var process=Process.Start(info)) {
            string output=process.StandardOutput.ReadToEnd()+process.StandardError.ReadToEnd();
            process.WaitForExit();
            int result=blocked ? (process.ExitCode==0 ? 91 : 0) : process.ExitCode;
            File.WriteAllText(Environment.GetEnvironmentVariable("BB_TEST_READY"),result+"\nCLI_EXIT="+process.ExitCode+"\n"+output);
            if(result!=0)return result;
        }
        using(var done=new EventWaitHandle(false,EventResetMode.ManualReset,Environment.GetEnvironmentVariable("BB_TEST_EVENT"))) {done.WaitOne();}
        return 0;
    }
}
'@)
$fixture=Join-Path $scratch 'hold.exe'
& "$env:SystemRoot\Microsoft.NET\Framework64\v4.0.30319\csc.exe" /nologo /target:exe "/out:$fixture" $helper
if($LASTEXITCODE -ne 0){throw 'Compile fixture failed'}
function Start-QueuedSession([string]$Operation,[bool]$Blocked=$false) {
    $key=[Guid]::NewGuid().ToString('N')
    $event=[Threading.EventWaitHandle]::new($false,'ManualReset',('Local\Blueberry.Test.'+$key))
    $ready=Join-Path $scratch ($key+'.ready')
    $info=[Diagnostics.ProcessStartInfo]::new($exe,('run --host-mode direct --no-profile --shell "'+$fixture+'" --data-dir "'+(Join-Path $scratch $key)+'"'))
    $info.UseShellExecute=$false; $info.CreateNoWindow=$true
    $info.RedirectStandardOutput=$true; $info.RedirectStandardError=$true
    $info.EnvironmentVariables['BLUEBERRY_NO_HISTORY']='1'
    $info.EnvironmentVariables['BB_TEST_EXE']=$exe
    $info.EnvironmentVariables['BB_TEST_OPERATION']=$Operation
    $info.EnvironmentVariables['BB_TEST_PACKAGE']=$Package
    $info.EnvironmentVariables['BB_TEST_READY']=$ready
    $info.EnvironmentVariables['BB_TEST_EVENT']='Local\Blueberry.Test.'+$key
    $info.EnvironmentVariables['BB_TEST_BLOCK_JOB']=if($Blocked){'1'}else{'0'}
    $process=[Diagnostics.Process]::Start($info)
    $deadline=[DateTime]::UtcNow.AddSeconds(60)
    while(-not [IO.File]::Exists($ready)) {
        if($process.HasExited -or [DateTime]::UtcNow -ge $deadline) { $null=$event.Set(); throw ('Session failed: '+$process.StandardError.ReadToEnd()) }
        Start-Sleep -Milliseconds 100
    }
    if(-not ([IO.File]::ReadAllText($ready)).StartsWith("0`n")) { $null=$event.Set(); throw ([IO.File]::ReadAllText($ready)) }
    return @{process=$process;event=$event}
}
function End-Session($Session) {
    $null=$Session.event.Set()
    if(-not $Session.process.WaitForExit(10000)){throw 'Fixture session did not exit naturally'}
    $Session.event.Dispose();$Session.process.Dispose()
}
try {
    $before=(Get-FileHash (Join-Path $root 'install.json')).Hash
    $session=Start-QueuedSession 'upgrade'
    try {
        $null=Wait-State @('waiting')
        if((Get-FileHash (Join-Path $root 'install.json')).Hash -ne $before){throw 'Upgrade committed while session was active'}
        & $exe upgrade --package $Package *> (Join-Path $scratch 'concurrent.log')
        if($LASTEXITCODE -eq 0 -or (Read-State).status -ne 'waiting'){throw 'Concurrent request changed the pending operation'}
        Invoke-Cli -Arguments @('maintenance','cancel')
        $null=Wait-State @('cancelled')
    } finally {End-Session $session}
    $session=Start-QueuedSession 'upgrade'
    $second=Start-QueuedSession 'hold'
    try {
        try { $null=Wait-State @('waiting') } finally {End-Session $session}
        if ((Read-State).status -ne 'waiting') { throw 'Maintenance did not wait for the second session' }
    } finally {End-Session $second}
    $null=Wait-State @('completed')
    $manifest=Get-Content (Join-Path $root 'install.json') -Raw|ConvertFrom-Json
    if(@($manifest.previous).Count -ne 1){throw 'Queued upgrade did not preserve the previous snapshot'}
    # Model exit after metadata replacement but before marking the journal committed.
    $maintenance=Join-Path $PSScriptRoot 'maintenance.ps1'
    $state=Read-State
    $journal=Join-Path $stateRoot ('transaction-'+$state.id+'/transaction.json')
    & {
        param($Manager,$Root,$Journal)
        . $Manager -Action Upgrade -FunctionsOnly -InstallRoot $Root
        $data=Read-JsonHashtable $Journal
        $data.outcome='prepared'; $data.committed_manifest_sha256=$null
        Write-Utf8JsonAtomic -Path $Journal -Value $data
    } $manager $root $journal
    $state.status='running'
    [IO.File]::WriteAllText($statePath,($state|ConvertTo-Json -Depth 20),[Text.UTF8Encoding]::new($false))
    & $maintenance -Mode Worker -InstallRoot $root -StateRoot $stateRoot
    if((Read-State).status -ne 'completed'){throw 'Post-commit interruption recovery failed'}
    Invoke-Cli -Arguments @('rollback')
    $null=Wait-State @('completed')

    $session=Start-QueuedSession 'upgrade' $true
    try {
        if((Read-State).status -ne 'queued'){throw 'A non-breakaway Job did not preserve the queued operation'}
    } finally {End-Session $session}
    $session=Start-QueuedSession 'hold'
    try { $null=Wait-State @('waiting') } finally {End-Session $session}
    $null=Wait-State @('completed')

    $before=(Get-FileHash (Join-Path $root 'install.json')).Hash
    $session=Start-QueuedSession 'upgrade'
    try {
        $state=Wait-State @('waiting')
        [IO.File]::AppendAllText($state.package,'corrupted after staging')
    } finally {End-Session $session}
    $null=Wait-State @('failed')
    if((Get-FileHash (Join-Path $root 'install.json')).Hash -ne $before){throw 'Tampered package changed installation'}

    # Deterministically model a worker killed after a file replacement but
    # before metadata commit, then recover in a fresh PowerShell invocation.
    & $maintenance -Mode Prepare -InstallRoot $root -StateRoot $stateRoot -Operation Upgrade -PackagePath $Package
    $state=Read-State
    $transaction=Join-Path $stateRoot ('transaction-'+$state.id)
    $readme=Join-Path $root 'README.md'
    $originalReadme=(Get-FileHash $readme).Hash
    & {
        param($Manager,$Root,$Directory)
        . $Manager -Action Upgrade -FunctionsOnly -TransactionDirectory $Directory -InstallRoot $Root
        $state=Read-InstallManifest -Root $Root -Required
        $transaction=New-Transaction -Root $Root -State $state
        Add-TransactionTouched $transaction 'README.md'
        [IO.File]::WriteAllText((Join-Path $Root 'README.md'),'interrupted write')
    } $manager $root $transaction
    $state.status='running'
    [IO.File]::WriteAllText($statePath,($state|ConvertTo-Json -Depth 20),[Text.UTF8Encoding]::new($false))
    & $maintenance -Mode Worker -InstallRoot $root -StateRoot $stateRoot
    if((Read-State).status -ne 'failed' -or (Get-FileHash $readme).Hash -ne $originalReadme){throw 'Interrupted transaction recovery failed'}
    [IO.File]::WriteAllText((Join-Path $root 'user-data.txt'),'keep')
    [IO.File]::WriteAllText($readme,'user edited documentation')
    $session=Start-QueuedSession 'uninstall'
    try { $null=Wait-State @('waiting') } finally {End-Session $session}
    $null=Wait-State @('completed')
    if((Test-Path $exe) -or (Get-Content $readme -Raw) -ne 'user edited documentation' -or (Get-Content (Join-Path $root 'user-data.txt') -Raw) -ne 'keep'){throw 'Uninstall did not preserve user files'}
    Write-Host "Maintenance lifecycle passed; retained evidence: $scratch"
} catch { Write-Host "Maintenance failure evidence retained: $scratch"; throw }
