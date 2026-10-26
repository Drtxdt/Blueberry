# vt100 0.16.2 local patch

The registry version, MIT license and metadata are preserved. The behavior change
is in `src/row.rs`: `Row::resize` clears a trailing wide-character leader after
shrinking, matching `Row::truncate`. Otherwise the next write dereferences a
missing continuation cell and panics. Regressions cover normal/alternate screens
and widths 1–5 in Blueberry overlay tests, plus the real tools UI resize fixture.

`screen.rs` and `perform.rs` additionally observe DECCRA (`CSI ... $ v`):
rectangular copies retain complete cells, support overlapping rectangles and
off-screen pages 2–6, and leave the cursor unchanged. Alternate screens use a
single page. Background pages resize without reflow, matching Windows Terminal.
This is needed because ConPTY passes these commands through; ignoring them gives
the probe a stale menu even when the real terminal restores the saved rectangle.
Independent protocol cases live in `tests/terminal_rectangles.rs` in Blueberry.
