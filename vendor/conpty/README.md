# Pinned Windows pseudoterminal runtime

`upstream.json` pins the official Microsoft.Windows.Console.ConPTY NuGet
package and both x64 payload digests. `scripts/prepare-conpty.py` downloads and
verifies it before a Windows build; `build.rs` independently verifies and embeds
the two payloads. No runtime download or machine-wide installation is involved.

The package declares MIT and Microsoft copyright. `LICENSE.txt` is the upstream
license from https://raw.githubusercontent.com/microsoft/terminal/2992421/LICENSE
(the input wakeup fix merge); the binary package version and hashes, rather than
that license source revision, identify the distributed binaries.

When a process first creates a PTY, Blueberry reuses or materializes the payloads
under the user's temporary `blueberry-runtime/conpty/<package-sha256>` directory.
It verifies both files while holding handles that deny writes and deletion,
loads the DLL by absolute path with system-only dependency lookup, and checks the
actual loaded path and required exports. A conflicting preloaded DLL is an error;
there is no silent fallback to the inbox host. portable-pty 0.9 then reuses this
module. The reference and file handles live for the process lifetime.

Normal direct sessions inherit their console and never call this loader.
Nested sessions and all Blueberry PTY probes use it. `doctor --json` publishes
the build pin; a nested session's adapter status and measurement reports also
record the loaded paths and identity. The final package manifest binds the pin
to its EXE, and formal evidence rejects a missing or mismatched runtime.

This addresses the observed class of stalls where the PSReadLine reader remains
in native `ReadConsoleInput` with complete key records already queued. The
upstream locking defect and fix are documented at
https://github.com/microsoft/terminal/pull/18816. Repeated runtime A/B tests and
complete terminal regressions remain necessary; a successful repetition does
not replace performance or functional qualification.
