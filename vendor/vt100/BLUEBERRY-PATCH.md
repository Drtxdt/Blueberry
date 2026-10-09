# vt100 0.16.2 local patch

The registry version, MIT license and metadata are preserved. The behavior change
is in `src/row.rs`: `Row::resize` clears a trailing wide-character leader after
shrinking, matching `Row::truncate`. Otherwise the next write dereferences a
missing continuation cell and panics. Regressions cover normal/alternate screens
and widths 1–5 in Blueberry overlay tests, plus the real tools UI resize fixture.
