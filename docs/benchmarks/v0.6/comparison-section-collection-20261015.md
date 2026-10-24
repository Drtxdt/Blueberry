# Bounded comparison collection

The `cb4-1014` coordinator disappeared without a final receipt after about 58
minutes. Its last progress was root pair 21/30. Inspection confirmed both PID
and creation-time identities were gone. The cause is unknown; it was not a
recorded six-hour timeout. All partial data remains available and ineligible
for the version decision.

`run-comparison.py` now supports independently owned `capability`, `startup`
and `hot` sections. Existing `full` behavior remains the default. A hot section
uses one scenario and an explicit, non-overlapping range of ten-query session
pairs. Use at most five pairs per execution session, then wait for completion
before starting the next. Keep the executing tool session held and inspected;
do not rely on an unverified detached launcher.

Capability evidence must come from a complete section with the same source,
products, observer, collector, plan, fixtures, profiles and power policy. Every
capability probe is hashed and revalidated, including supported capabilities.
Each section verifies dependencies at both boundaries. A formal section still
records `measurement_complete=false`; `section_complete` cannot qualify the
whole matrix. Interrupted sections are preserved and replaced by independent
new sections, never resumed by appending arrays.

`merge-comparison-sections.py --reports <all section comparison.json paths>
--output <new report>` requires exactly one capability section, one complete
startup section, and disjoint hot ranges covering all six scenarios. It checks
all identities and raw arrays, recomputes each shard statistic, then recomputes
aggregate statistics without removing outliers. Missing ranges, overlaps,
changed dependencies, invalid input evidence or corrupt statistics fail the
merge. Only a complete merge sets `measurement_complete=true`. Pixel timing
and complete-candidate timing remain explicitly unmeasured in this four-way
observer; separate direct measurements retain their own acceptance rules.

Validation: 12 comparison-script tests passed locally, including complete raw
array recomputation, missing/overlapping shards, identity mismatch, insufficient
or incomplete evidence, and changed dependencies. A new real small-sample
capability/startup/hot preflight is kept under `artifacts/cb5-1015`; its outcome
must be recorded before formal collection. No native-overlay performance result
is implied by a successful legacy observer preflight.
