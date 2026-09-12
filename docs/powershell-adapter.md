# PowerShell adapter

`shell/integration.ps1` is dot sourced by the one PowerShell process owned by
shellsense. A normal launch keeps the user's profile enabled and then runs:

```powershell
pwsh -NoLogo -NoExit -Command ". 'C:\path\to\integration.ps1'"
```

The host supplies `SHELLSENSE_TOKEN` and `SHELLSENSE_EDIT_PATH` in the child
environment. The adapter reads the token once and removes it from the process
environment. Every protocol frame is written directly to stdout as:

```text
ESC ] 7776 ; token ; compact-json BEL
```

The JSON object always has an `event` property. The adapter emits these
events:

| Event | Additional properties |
| --- | --- |
| `capabilities` | `ready` (all PSReadLine handlers registered), `psreadline`, `key_handlers` (`buffer`, `apply`, `commands`), `edit_path` |
| `prompt_start` | `cwd` |
| `prompt_end` | `cwd`, `path`, `pathext`, `pid` |
| `execute` | none |
| `buffer` | `line`, `cursor` (UTF-16 code-unit offset) |
| `commands` | `snapshot` UUID, `complete` boolean, `commands` (up to 128 records), each with `name`, `kind`, and `definition` (aliases include their target) |
| `trace` | Optional fixed `stage` and numeric `duration_ms`; enabled only by the host trace switch |
| `error` | `code`, `message`, and sometimes `chord` |

On hosts that load PSReadLine just before the first prompt, the initial
capabilities frame may report `ready: false`; the adapter registers the
handlers from that first prompt and sends one refreshed capabilities frame.

When PSReadLine is available, `F12,s` reports its current buffer,
`F12,a` applies one pending edit, and `F12,c` reports the loaded aliases,
functions, and cmdlets. The edit file is atomically renamed before it is read
and removed after handling, so the same payload cannot be replayed. Its
`expectedLine` and `expectedCursor` must match the live buffer exactly, and
`start`/`length` are checked as UTF-16 code-unit ranges that cannot split a
surrogate pair. Replacement text is passed directly to PSReadLine's
`Replace` method and is never evaluated as PowerShell.

Repeated `F12,c` calls continue the same immutable snapshot until
`complete: true`; the next call starts a new snapshot. There is no 512-command
limit. The host prioritizes buffer queries between snapshot pages. It keeps
previous session commands until the last page, then replaces the old snapshot
so removed aliases/functions disappear. Legacy unbatched frames are still
accepted as complete.

Serialization and edit parsing use PowerShell's existing `System.Text.Json`.
The cached default encoder preserves ASCII OSC transport and Unicode surrogate
pairs. Startup uses the session-variable and prompt APIs, and PSReadLine's
direct key-handler API, without importing Utility solely for JSON. The adapter
checks both reserved composite chords and an existing bare F12 binding.

If a reserved chord is already bound, the existing binding is left unchanged
and an `error` event with code `key_chord_collision` is sent. If PSReadLine is
not present in a noninteractive process, it is not auto-loaded; the readiness
event reports `psreadline: false`.

Set `SHELLSENSE_NO_HISTORY=1` for isolated probes or CI sessions that should
leave no PSReadLine history file behind. Ordinary launches leave PSReadLine's
history settings unchanged.

The token-bearing bootstrap switches the child console input/output to UTF-8.
Protocol JSON additionally escapes non-ASCII characters to protect the OSC
transport if a later command changes the console code page. A code-page change
can still affect PSReadLine's visible redraws. This integration never edits
profile files or global console settings.

The prompt wrapper invokes the original prompt before transport helpers, so
the prompt sees the preceding command's success status. LASTEXITCODE is read
through the session-variable API and restored, allowing StrictMode before any
native command has populated that variable.
