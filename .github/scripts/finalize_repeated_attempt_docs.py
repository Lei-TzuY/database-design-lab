from pathlib import Path

roadmap = Path('docs/roadmap.md')
text = roadmap.read_text()
old = '''- [ ] Complete recovery-cost and compaction-stall distributions. Successful samples now pair duration with
  the exact measured trace-step index and deterministic data-path work: B+ tree reopen page accesses/bytes,
  LSM reopen WAL+SSTable record versions/bytes, and LSM full-set compaction input record versions/bytes. Whole-run
  execution order is explicit, fresh AB/BA pairs are first-class in `db-core`, and
  `experiment-archive-counterbalanced` preserves both repetitions plus pair-order provenance. Non-success
  invocations are now retained as immutable format-v3 `failed`/`excluded` attempt evidence rather than being
  silently dropped. An enforced cache/filesystem preparation protocol is still required before this item is complete.
'''
new = '''- [ ] Complete recovery-cost and compaction-stall distributions. Successful samples pair duration with exact
  measured trace-step indices and deterministic data-path work. Whole-run order is explicit, fresh AB/BA pairs
  are first-class, and one counterbalanced invocation is archived as successful format-v2 evidence or immutable
  format-v3 `failed`/`excluded` attempt evidence. The reusable repeated-batch layer additionally alternates outer
  pair order from a recorded seed and retains every requested pair as included, failed, or explicitly excluded,
  continuing after pair failure; factory failures carry left/right plus repetition provenance. An enforced
  cache/filesystem preparation protocol, immutable archive wiring for the repeated-batch ledger, and duration/work
  capture for failed internal recovery or compaction operations remain required before this item is complete.
'''
assert old in text
roadmap.write_text(text.replace(old, new))

method = Path('docs/amplification-methodology.md')
text = method.read_text()
old = '''The compatibility duration vectors and structured vectors are appended together and tests require their
indices/durations to agree. Successful-sample work accounting is therefore deterministic and trace-associated,
but the roadmap item remains incomplete: failed/excluded recovery or compaction attempts are not retained,
engine execution order is not counterbalanced, and the archive's declared cache/filesystem state is not an
enforced protocol. Scheduler noise, build profile, host identity, cache state, filesystem, and storage device
must still be controlled before timing distributions can support a performance claim.
'''
new = '''The compatibility duration vectors and structured vectors are appended together and tests require their
indices/durations to agree. Successful-sample work accounting is therefore deterministic and trace-associated.
Whole-engine ordering is explicit: ordered comparisons run one candidate's complete setup/measured window before
the other, and a counterbalanced pair uses four fresh engine instances to execute one AB and one BA run.

## Repeated-attempt ledger and exclusion boundary

The counterbalanced publication path and the reusable repeated-sampling layer intentionally solve different
provenance problems. `experiment-archive-counterbalanced` retains one invocation as backward-compatible format-v2
success evidence; an execution failure or caller-requested methodological exclusion is retained as immutable
format-v3 attempt evidence instead of disappearing from the run-level denominator.

`run_counterbalanced_experiment_batch` sits above one fresh AB/BA pair. The low bit of a recorded `pair_seed`
chooses the first outer pair order and later requested pairs alternate deterministically. Every requested pair has
a zero-based index and one of three dispositions: `included`, `failed`, or `excluded`. Included entries retain the
complete counterbalanced pair report. Failed entries retain a stable `ErrorClass` and diagnostic text; fresh-engine
factory failures additionally identify left/right role and repetition 0/1, while later comparison/runtime failures
are labeled `comparison` without inventing side attribution. Exclusions happen before engine creation and require a
non-empty reason. One failed pair does not abort later requested pairs, preventing harness control flow from
silently turning a requested batch into a success-only sample set.

The repeated batch ledger itself is not yet written by an immutable archive command, and engine-local timing still
records successful REOPEN/compaction samples only: no duration/work sample is retained for an individual failed
REOPEN or compaction operation. Cache/filesystem state also remains declared metadata rather than an enforced
preparation protocol. Scheduler noise, build profile, host identity, cache state, filesystem, and storage device
must therefore still be controlled before timing distributions can support a performance claim.
'''
assert old in text
method.write_text(text.replace(old, new))
