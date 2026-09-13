# PowerShell adapter

`shell/integration.ps1` is dot sourced by the one PowerShell process owned by
shellsense. A normal launch keeps the user's profile enabled and then runs:

```powershell
pwsh -NoLogo -NoExit -Command ". 'C:\path\to\integration.ps1'"
```

The host supplies `SHELLSENSE_TOKEN` and `SHELLSENSE_EDIT_PATH` in the child
environment. It may also provide `SHELLSENSE_REQUEST_PATH` and
`SHELLSENSE_KEY_PREFIX` (`F5` through `F12`; the alpha default is `F12`). For
public shortcut arbitration it may provide `SHELLSENSE_PUBLIC_KEYS` as a JSON
object with `trigger`, `native`, `details`, `refresh`, and `reload` chord
strings. The adapter reads the token once and removes it from the process
environment. `request.json` defaults to the sibling of `edit.json`. Every
protocol frame is written directly to stdout as:

```text
ESC ] 7776 ; token ; compact-json BEL
```

The explicit `run --transport pipe` experiment also supplies
`SHELLSENSE_PIPE_NAME`. The adapter creates a local duplex
`NamedPipeClientStream` and attempts `Connect(0)` once during bootstrap. A
failed connection keeps OSC active. In pipe mode an event is first written as
one UTF-8 message-mode frame with the envelope
`{"sequence":N,"payload":{...}}` and then acknowledged on stdout by the
short OSC barrier `{"event":"pipe","sequence":N}`. The host consumes the
matching sequence before processing the payload, preserving output and cursor
ordering. A frame larger than 1 MiB or any pipe write failure is sent as the
original OSC event after disposing the client stream. The default transport is
still OSC, and this experiment does not claim a performance improvement.

Host requests and edits use the opposite pipe direction as one frame of the
form `{"kind":"request"|"edit","payload":{...}}`; the known `${prefix},n`,
`${prefix},a`, and `${prefix},c` handlers read only the corresponding already
published message. In pipe mode a command page after the first one carries a
`commands_next` request before `${prefix},c`. The adapter has no reader thread
and does not poll the pipe between keys. If the client disconnects, subsequent
events use OSC and file payloads as in the legacy path.

The JSON object always has an `event` property. The adapter emits these
events:

| Event | Additional properties |
| --- | --- |
| `capabilities` | `protocol_version: 2`, legacy `ready`/`psreadline`/`key_handlers`/`edit_path`, `key_prefix`, actual `transport` (`osc` or `pipe`), request-path state, and a v2 `capabilities` map. `key_handlers` includes `native` and optional `enter`/`shift_enter`; when `SHELLSENSE_PUBLIC_KEYS` is set, `capabilities.public_keys` reports each shortcut's safe-to-intercept status. |
| `prompt_start` | `cwd` |
| `prompt_end` | `cwd`, `path`, `pathext`, `pid`, and a transient `environment` map when it changes |
| `buffer` | `line`, `cursor` (UTF-16 code-unit offset); complex lines additionally carry a compact `context` (see below) |
| `editing` | `state: continuation`, only after a known PSReadLine `AcceptLine`/`AddLine` action leaves a live continuation buffer |
| `execute` | Emitted only when the wrapped `PSConsoleHostReadLine` actually returns an accepted command |
| `edit_result` | `request_id` (nullable for legacy payloads), `applied` after `PSConsoleReadLine.Replace` succeeds or fails |
| `native_completion` | `request_id`, live `line`/`cursor`, UTF-16 `replace_start`/`replace_end`, `candidates`, and `status` |
| `commands` | `snapshot` UUID, `complete` boolean, and a batch of at most 64 records or roughly 2 ms of same-runspace enumeration. Each record has `name`, `kind`, and `definition` (aliases include their target). |
| `trace` | Optional fixed `stage` and numeric `duration_ms`; enabled only by the host trace switch. Startup stages include `adapter_bootstrap`, `readline_init`, `key_snapshot`, `key_register`, `public_keys`, and `readline_wrap`; runtime stages include `command_snapshot` and `serialize` |
| `error` | `code`, `message`, and sometimes `chord` |

On hosts that load PSReadLine just before the first prompt, the initial
capabilities frame may report `ready: false`; the adapter registers the
handlers from that first prompt and sends one refreshed capabilities frame.

When PSReadLine is available, `${prefix},s` reports its current buffer,
`${prefix},a` applies one pending edit, `${prefix},c` reports the loaded aliases,
functions, and cmdlets, and `${prefix},n` requests native completion. The
optional `${prefix},e` and `${prefix},l` chords are installed only when Enter
and Shift+Enter still use PSReadLine's built-in `AcceptLine` and `AddLine`
handlers. A custom binding is preserved and reported as an unavailable
lifecycle capability. The edit file is atomically renamed before it is read
and removed after handling, so the same payload cannot be replayed. Its
`expectedLine` and `expectedCursor` must match the live buffer exactly, and
`start`/`length` are checked as UTF-16 code-unit ranges that cannot split a
surrogate pair. An optional string `id` is echoed as `request_id` in
`edit_result`. Replacement text is passed directly to PSReadLine's `Replace`
method and is never evaluated as PowerShell. The acknowledgement is emitted
before the follow-up live `buffer` event, and is emitted only after Replace has
returned or thrown.

Native completion is manual-only. The adapter reads a request object such as
`{"id":"req-17","kind":"native"}` from `request.json`, calls
`CommandCompletion.CompleteInput` in the current runspace, and returns its
single replacement range without mixing Rust candidates into the response.
Native completers may execute user code, so the adapter does not invoke this
path from automatic buffer queries or promise a forced cancellation. A
`{"id":"req-18","kind":"commands_reset"}` request resets the lazy command
enumerator before the next command batch; an accepted command resets it too.
The reset request may include `public_keys` as an object or
`public_keys_json` as a serialized object. When present, the adapter rechecks
the current PSReadLine owners and emits a fresh `capabilities` frame before
the first batch. Requests without that field preserve the existing shortcut
arbitration state.

For a complex PowerShell line, `buffer.context` contains only compact data:
`command`, preceding `arguments` as strings (the current token is excluded),
decoded current `prefix`, absolute UTF-16 `replace_start`/`replace_end`,
`command_position`, `suppressed`, and `complex: true`. Ordinary lines omit the
context field. Comments, here-string bodies, variables outside the known
`$env:` namespace, and other expressions whose value cannot be proven safe
set `suppressed: true`; the adapter never evaluates them. The same context
may accompany `native_completion` to let the host attach known Chinese
descriptions.

Repeated `${prefix},c` calls continue the same immutable snapshot until
`complete: true`; the next call starts a new snapshot. The adapter holds the
PowerShell enumerator itself and advances it in the live runspace, so it never
first collects the complete command list. A partial page is always marked
`complete: false`; exceptions cannot publish a complete marker. The host
prioritizes buffer queries between pages. It keeps previous session commands
until the last page, then replaces the old snapshot so removed
aliases/functions disappear. Legacy unbatched frames are still accepted as
complete.

`prompt_end` retains the legacy `path` and `pathext` fields and may include a
transient `environment` name/value map when it changes. The map is held only
by the host for project-aware completion; it is never written to trace,
diagnostics, command history, or persistent command caches.

Serialization and edit parsing use PowerShell's existing `System.Text.Json`.
The cached default encoder preserves ASCII OSC transport and Unicode surrogate
pairs. Startup uses the session-variable and prompt APIs, and PSReadLine's
direct key-handler API, without importing Utility solely for JSON. The adapter
checks both reserved composite chords and an existing bare protocol-prefix binding (F12 by default).

If a reserved chord is already bound, the existing binding is left unchanged
and an `error` event with code `key_chord_collision` is sent. If PSReadLine is
not present in a noninteractive process, it is not auto-loaded; the readiness
event reports `psreadline: false`.

When `SHELLSENSE_PUBLIC_KEYS` is present, the adapter also checks those host
shortcuts against the current PSReadLine bindings. An unbound shortcut is
reported as available. The completion `trigger` is available when its existing
binding is the standard `MenuComplete` or `Complete` action, and `details` is
available with the standard `ShowCommandHelp` action because the host only
uses it while its menu is visible. A custom binding, including a ScriptBlock,
is preserved and reported as a `public_key_collision` error. Other bound public
shortcuts are likewise reported unavailable, with a key-setting suggestion.
The adapter never replaces these bindings. Omitting the variable omits
`capabilities.public_keys` for compatibility with older hosts.

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
