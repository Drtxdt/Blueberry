# Session-private PSReadLine

`upstream.json` fixes the source commits and archive SHA-256 values for 2.0.0
and 2.4.5. The MIT license is preserved in `LICENSE.txt` and each embedded
module. `patches/` changes only the upstream editor loop; `EditorIntegration.cs`
implements the common version 1 contract. Without a registration the original
editor loop and key handlers remain in control.

On Windows, install .NET SDK 8.0.425, Python 3 and Git, then run:

```powershell
python scripts/prepare-editor.py
cargo build --release --locked
```

The preparation script verifies source archives, applies the reviewable patch,
builds both modules, and records every shipped file digest in
`target/private-editor/manifest.json`. Cargo rejects a missing or stale manifest
or any changed asset. Compilation never happens at product startup. Runtime
assets are installed atomically in a content-addressed session resource cache.
`blueberry doctor --json` includes the embedded module identities under
`build.private_editors`, and the active module identity under `editor`.

`blueberry run --host-mode direct --psreadline-version auto|2.0.0|2.4.5`
selects the private baseline. `auto` uses supported module discovery; a loaded
assembly is never replaced. A different loaded module or missing interface
uses the existing compatibility adapter and records the fallback reason.
The Windows default host is `direct`; `--host-mode nested` remains available.

The refresh signal is an AutoResetEvent owned for the PSReadLine singleton's
lifetime. Only the outer input loop waits on it. Nested key reads (search,
chords, numeric arguments) and Vi command mode do not enter refresh callbacks.
The editor thread owns callbacks, buffer reads, replacement and console output.
Callback exceptions unregister the integration. A read-line session ends before
command execution; revisions reject results from preceding prompts.

The file bootstrap first prepares only bridge/protocol methods while the shell
loads module options. This pass never touches the editor type. After assembly
identity verification and normal shell-thread type initialization, a finite
background pass asks the CLR to
prepare selected initialization, editing and painting methods. It invokes no
editor methods and performs no console I/O; normal CLR compilation remains the
fallback. The public `ReadLine` entry and console initialization are prepared
before rendering helpers. This overlaps first-use JIT work with the remainder
of shell startup without moving editor initialization ahead of profiles.
It is not an idle polling thread. Isolated probes can compare it with
`BLUEBERRY_TEST_DISABLE_PREJIT=1`; formal matrices reject that override.

Incoming ordinary frames are coalesced only after revision/frame-order checks.
Workbench frames and edit/control replies retain their receive order. An edit
requires either the displayed candidate's pending acceptance identity or the
current explicit form confirmation, as well as the expected buffer and cursor.
Invalid replies leave the current visible candidate intact.

The direct renderer ends its single menu/cursor write with the inert private
OSC 1337 action `BlueberryFrame`. Legacy ConPTY synchronously flushes the real
backing buffer before forwarding OSC 1337 actions. Terminals that do not know
this action ignore it; it requests no reply and changes no input mode. Newer
ConPTY implementations pass output through directly. The isolated
`console_output_transport_experiment` test compares APIs, batch-only output and
this boundary; its result is diagnostic and cannot replace complete-menu timing.

Performance qualification requires trace-disabled measurements of the final
EXE, its embedded DLL digests, and the same probe and fixtures. Functional test
timeouts and small delay-injection samples are not performance acceptance.
Windows Terminal IME/clipboard/visual checks remain separate required evidence.
