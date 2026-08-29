# Repeated counterbalanced batch evidence

`db-lab-batch` is the immutable archive path for repeated exploratory Phase 4 experiments. It turns the reusable `db-core` repeated-batch ledger into a portable evidence bundle without discarding unsuccessful or deliberately excluded pairs.

## Execution and retention contract

One invocation receives a validated experiment trace, a recorded `pair_seed`, and `--pairs N`. The seed's low bit selects the first pair's outer order and later pair indices alternate that order deterministically. Every included pair still contains one left-then-right and one right-then-left whole-engine execution.

Every fresh engine instance is created beneath a new `--engine-root` using the stable layout
`pair-{pair_index:06}/repetition-{repetition_index}/{btree.db|lsm}`. `--engine-root` and `--archive-dir` must both be absent before the invocation and may not be nested inside each other.

Pairs may be excluded before engine creation with repeated `--exclude-pair INDEX=REASON` arguments. Reasons are trimmed, bounded to 4 KiB, unique by pair index, and retained verbatim in the batch ledger. Runtime or factory failures are retained by `db-core` as failed attempts and do not prevent later pairs from running.

The command writes the complete archive before returning a non-zero status for retained failed pairs. This makes automation notice the failed experiment while preserving the denominator and diagnostic evidence.

## Format v6

A successful archive write creates four immutable JSON files:

- `trace.json` — the exact validated input trace;
- `batch.json` — the complete `CounterbalancedExperimentBatchReport`, including every included, failed, and excluded requested pair;
- `environment.json` — source revision, pair seed/count, engine layout, build/target/rustc identity, declared host/filesystem/storage/cache metadata, B+ tree cache capacity, timestamp, and notes;
- `index.json` — format version, repository revision, protocol identifiers, and the exact archive file list.

The stable protocol identifiers are:

- `execution_protocol = "fresh_counterbalanced_repeated_batch_v1"`;
- `attempt_protocol = "retain_all_requested_pairs_v1"`;
- `format_version = 6`.

Existing paths are never overwritten. Partial archive directories are removed if serialization or durable file creation fails.

## Methodology boundary

Format v6 is an **exploratory repeated-batch archive**, not automatic publication admission. The `publication_warm_v1` gate on `db-lab experiment-archive-counterbalanced` remains the stricter single-pair publication path. A later change may share that admission record with repeated batches once the batch-level controlled-host and analysis protocol is frozen.

Likewise, retaining a failed pair does not yet retain a duration/work sample for an individual failed internal REOPEN or compaction operation. That lower-level failure instrumentation remains a separate Phase 4 requirement.
