$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$env:BLUEBERRY_TOKEN = 'pipe-deadline-test'
$env:BLUEBERRY_DIRECT_PIPE = '1'
$savedOut = [Console]::Out
[Console]::SetOut([IO.StringWriter]::new())
try { . (Join-Path $PSScriptRoot '../shell/integration.ps1') }
finally { [Console]::SetOut($savedOut) }

# Model a cold method binding / scheduler pause before the operating-system
# operation starts. The pipe and its payload are real, not mocked task results.
Add-Type -TypeDefinition @'
using System;
using System.IO.Pipes;
using System.Threading;
using System.Threading.Tasks;
public sealed class DelayedPipe : IDisposable {
    readonly NamedPipeClientStream stream;
    public DelayedPipe(NamedPipeClientStream stream) { this.stream = stream; }
    public bool IsConnected { get { return stream.IsConnected; } }
    public bool IsMessageComplete { get { return stream.IsMessageComplete; } }
    public Task<int> ReadAsync(byte[] bytes, int offset, int count, CancellationToken token) {
        Thread.Sleep(100);
        return stream.ReadAsync(bytes, offset, count, token);
    }
    public Task WriteAsync(byte[] bytes, int offset, int count, CancellationToken token) {
        Thread.Sleep(100);
        return stream.WriteAsync(bytes, offset, count, token);
    }
    public void Dispose() { stream.Dispose(); }
}
'@

foreach ($case in @('read-ready', 'write-ready', 'read-empty')) {
    $name = 'blueberry-deadline-' + [Guid]::NewGuid().ToString('N')
    $server = [IO.Pipes.NamedPipeServerStream]::new($name, [IO.Pipes.PipeDirection]::InOut, 1,
        [IO.Pipes.PipeTransmissionMode]::Message, [IO.Pipes.PipeOptions]::Asynchronous)
    $client = [IO.Pipes.NamedPipeClientStream]::new('.', $name, [IO.Pipes.PipeDirection]::InOut,
        [IO.Pipes.PipeOptions]::Asynchronous)
    try {
        $connect = $server.WaitForConnectionAsync()
        $client.Connect(2000)
        $connect.GetAwaiter().GetResult() | Out-Null
        $client.ReadMode = [IO.Pipes.PipeTransmissionMode]::Message
        $script:BLUEBERRY_PIPE_STREAM = [DelayedPipe]::new($client)
        $script:BLUEBERRY_PIPE_ENABLED = $true
        if ($case -eq 'read-ready') {
            $bytes = [Text.Encoding]::UTF8.GetBytes('{"value":42}')
            $ready = $server.WriteAsync($bytes, 0, $bytes.Length)
            $result = Read-BlueberryPipeJson
            $value = $script:BLUEBERRY_JsonElement::new()
            if ($null -eq $result -or -not $result.TryGetProperty('value', [ref]$value) -or
                (Convert-BlueberryJsonElementValue $value) -ne 42) {
                throw 'Ready message was cancelled before ReadAsync began'
            }
            $ready.GetAwaiter().GetResult() | Out-Null
        } elseif ($case -eq 'write-ready') {
            $buffer = [byte[]]::new(4096)
            $pending = $server.ReadAsync($buffer, 0, $buffer.Length)
            if (-not (Send-BlueberryPipeEvent -Payload ([ordered]@{event='test'}))) {
                throw 'Writable pipe was cancelled before WriteAsync began'
            }
            if (-not $pending.Wait(2000)) { throw 'Pipe write never arrived' }
            $envelope = [Text.Encoding]::UTF8.GetString($buffer,0,$pending.Result) | ConvertFrom-Json
            if ($envelope.payload.event -ne 'test') { throw 'Pipe payload changed' }
        } else {
            $timer = [Diagnostics.Stopwatch]::StartNew()
            $result = Read-BlueberryPipeJson
            if ($null -ne $result -or $script:BLUEBERRY_PIPE_ENABLED -or $timer.ElapsedMilliseconds -gt 2000) {
                throw 'Empty pipe did not cancel and disconnect within the bound'
            }
        }
        Write-Output "$case passed"
    } finally {
        $client.Dispose()
        $server.Dispose()
    }
}
