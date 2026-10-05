# Private editor integration: exploratory evidence

This directory is **not release qualification**. Reports come from local dirty
builds and different implementation revisions; each JSON retains its own EXE
identity. Default hosting remains nested. Do not combine these samples into a
formal percentile or treat skipped tests as passed.

The implementation adds editor-thread callbacks and a dedicated wait event to
fixed upstream PSReadLine 2.0.0 and 2.4.5 sources. The direct bridge no longer
installs character bindings or polls a PowerShell object-event queue in this
mode. Its compatibility path remains available and reports degradation.

## What the measurements established

- r1 removed the roughly 300 ms idle-event delay but PS7 hot P95 still reached
  33 ms. Startup exploration was 40.13 ms paired P50 (10 pairs).
- Correlated r2 samples put receive-to-editor entry P95 at 0.649 ms, while
  console write-to-VT arrival still reached 16.752 ms.
- The output experiment compares console calls, batched VT output, and a
  single write ending in the inert OSC 1337 `BlueberryFrame` action. The last
  one flushes legacy ConPTY's actual backing buffer. This follows
  [the pinned Terminal implementation](https://github.com/microsoft/terminal/blob/295cd17b028d288ff81445f532d9301b44f6ffd9/src/terminal/adapter/adaptDispatch.cpp),
  not an assumption that Windows has a fixed timing floor.
- r3 passed 11/12 PS7 hot subgroups. Cargo miss still failed at 24.37 ms.
  Its slow samples primarily waited between injected input and editor
  confirmation. Draining keys already queued by the native reader removes
  redundant reader handoffs while preserving native key dispatch.
- r4 passed 12/12 PS7 hot subgroups at 30 samples each: complete-menu P95
  6.91–13.55 ms. This is exploratory, descriptions enabled only.
- r5 PS5.1 startup against the Windows inbox module labelled 2.0.0 failed
  at 73.74 ms P50. That inbox DLL is a different assembly lineage from the
  pinned upstream/PSGallery 2.0.0 (`PSReadLine.dll` versus `PSReadLine2.dll`).
  Against the official matching module, 10 pairs gave 38.86 ms P50, but the
  first plain launch took 2452 ms. Both reports are retained; neither is a
  final 30-pair result. New reports record the actual plain DLL identity.

## Subsequent startup and functional work

The latest local code is still blocked on startup qualification. These are
separate exploratory builds/batches, **not** a combined acceptance result:

| Evidence | Combination | Result |
| --- | --- | --- |
| `matrix-b7bfb4b` | PS5.1 / 2.0.0 | Startup paired P50 55.07 ms, failed |
| `matrix-r10` | PS5.1 / 2.0.0 | Startup 46.30 ms; all 24 hot subgroups passed at 30 samples each, P95 6.60–13.26 ms |
| `matrix-r10` | PS5.1 / 2.4.5 | Startup 62.54 ms, failed; matrix stopped |
| r12 same-EXE pre-JIT off/on | PS5.1 / 2.4.5 | Startup 51.34 / 45.53 ms; first-key P50 38.54 / 29.79 ms, 10 pairs per batch |
| `matrix-r12` | PS5.1 / 2.0.0 | Startup 51.51 ms, failed; matrix stopped |
| `matrix-r13` | PS5.1 / 2.0.0 | Startup 64.69 ms, failed; first-key P50 21.13 ms |
| `hot-ps51-245-r13.json` | PS5.1 / 2.4.5 | Separate hot regression: all 12 subgroups passed, 30 samples each, P95 6.83–12.15 ms, descriptions enabled |

r10's first hot invocation was interrupted. Its raw partial data is retained;
the successful descriptions-on batch is explicitly named `attempt-2`. Resume
is exploratory-only and checks artifact identities. The formal runner refuses
resume, diagnostic overrides, dirty builds, wrong original DLL identities and
undersized batches. Every failed gate stops the matrix and is retained.

The private-DLL control report loads the identical private 2.4.5 assembly in
the plain side without bridge registration. Its four deltas remained
50.57–90.77 ms; this diagnostic is not a substitute for the original-module
baseline. Assembly changes alone therefore do not explain the remaining cost.
The numeric traces now separate input-loop entry, begin completion, key
acquisition, native dispatch and native completion. A finite CLR method
preparation pass improves first-key latency in the r12 comparison but has not
established stable startup acceptance. The shell interval before bootstrap
still includes module and profile costs that need finer attribution.

WPR CPU/.NET recording could not start: Windows returned `0xc5585011` while
enabling the system performance profiling policy. A status check confirmed no
recording remained. A second attempt with the DotNET profile alone failed with
the same error and also left no recording. There is no scheduler or CLR ETW
evidence from either attempt. `startup-attribution-r14.json` retains the
available numeric stage breakdown and explicitly labels its unresolved spans.

The r13 direct logs contain **16 passed tests on each of the three combinations**,
with one ignored output experiment. Added coverage includes Vi command/insert
transitions, callback failure recovery, reordered and duplicate frames, wrong
candidate identities, unsolicited replacements, and reliable workbench/edit
queue ordering. The r14 pipe change additionally removes the five-second
connection deadline for a live shell: slow profiles can finish, while actual
child exit cancels connection/read/write waits. Its core log has 92 passing
library tests, including delayed connection and early child exit. Older
performance reports do not qualify this later pipe change. The later r14
PS5.1/2.4.5 direct regression has 16 passes; nested terminal-mode regression has
3 passes and 2 explicit skips (helper/native mouse capability).

Raw archives include fixtures, failure/partial logs and numeric traces where
available. The JSON reports retain EXE and private DLL identities, plain module
identities, profiles and power policy. All reports here remain local evidence;
no CI candidate, full formal matrix or manual Terminal sign-off exists yet.

## Regression scope and probe corrections

The r9 logs contain 14 direct tests passed on each of the three combinations;
the isolated output experiment is ignored in the ordinary suite. The tests
cover native editing equivalence, explicit UTF-16 Unicode key events, cursor
and selection edits, undo/redo, chords, digit arguments, delayed refresh,
rebinding, revision/acceptance behavior, forms, resize, disconnect, command
status, standard-module autoload and external command return. Earlier core and nested suites remain
separate evidence. These tests do not cover every item in the requested
manual and fault-injection matrix.

Earlier r6 failures are also retained. One wait incorrectly matched logical
VT lines whose wrap metadata survived an overlay; physical-row matching
observed the actual output. Unicode loss was reproduced in the unmodified
editor too. ConPTY's raw-text fallback converts input using its output code
page ([source](https://github.com/microsoft/terminal/blob/295cd17b028d288ff81445f532d9301b44f6ffd9/src/terminal/adapter/InteractDispatch.cpp)).
The native editing comparison now sends Unicode-only win32 input records,
without changing product input handling; ASCII/VT editing sequences remain
unchanged. The separate raw UTF-8 plain test remains. This does **not** certify
IME or clipboard behavior in a real Windows Terminal.

Local unchanged upstream tests executed only 6 (2.0.0) and 20 (2.4.5) tests;
176 and 564 respectively were skipped by the upstream keyboard-layout guards.
CI requires meaningful coverage and preserves TRX results. Local skip-heavy
runs are not evidence of full upstream compatibility.

## Outstanding qualification

- New clean CI package, matching probe and private-DLL identities.
- Three-combination exploration, then 30 startup pairs and at least 300
  samples in each scenario/cache/descriptions subgroup on that package.
- Full upstream test execution, remaining modal/lifecycle/fault regressions,
  and separate Windows Terminal IME, clipboard and visual evidence.
- Scheduler wakeup tracing and complete startup attribution, including the
  pre-bootstrap module/profile interval. Numeric current traces distinguish
  resource preparation, spawn, bootstrap, assembly, registration, handshake,
  editor confirmation, receive, wake, write and VT arrival; they do not yet
  individually instrument every profile phase.

Use `scripts/run-editor-matrix.ps1`; it retains failures and stops at the first
failed gate. `--trace` runs are diagnostic and cannot be formal timing runs.
The process-tree resource report explicitly distinguishes unavailable wakeup
counts from CPU measurements. `sha256.json` preserves the evidence copied
here; further results must use new names.
