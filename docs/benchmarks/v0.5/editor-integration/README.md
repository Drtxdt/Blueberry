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
