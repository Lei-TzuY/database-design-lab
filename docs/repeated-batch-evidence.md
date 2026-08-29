# Repeated counterbalanced batch evidence

`db-lab-batch` is the immutable archive path for repeated Phase 4 experiments. It turns the reusable `db-core` repeated-batch ledger into a portable evidence bundle without discarding unsuccessful or deliberately excluded pairs.

## Execution and retention contract

One invocation receives a validated experiment trace, a recorded `pair_seed`, and `--pairs N`. The seed's low bit selects the first pair's outer order and later pair indices alternate that order deterministically. Every included pair still contains one left-then-right and one right-then-left whole-engine execution.

Every fresh engine instance is created beneath a new `--engine-root` using the stable layout
`pair-{pair_index:06}/repetition-{repetition_index}/{btree.db|lsm}`. `--engine-root` and `--archive-dir` must both be absent before the invocation and may not be nested inside each other.

Pairs may be excluded before engine creation with repeated `--exclude-pair INDEX=REASON` arguments. Reasons are trimmed, bounded to 4 KiB, unique by pair index, and retained verbatim in the batch ledger. Runtime or factory failures are retained by `db-core` as failed attempts and do not prevent later pairs from running.

The command writes the complete archive before returning a non-zero status for retained failed pairs. This makes automation notice the failed experiment while preserving the denominator and diagnostic evidence.

## Exploratory format v6

The default `--admission exploratory` mode preserves format v6. A successful archive write creates four immutable JSON files:

- `trace.json` — the exact validated input trace;
- `batch.json` — the complete `CounterbalancedExperimentBatchReport`, including every included, failed, and excluded requested pair;
- `environment.json` — source revision, pair seed/count, engine layout, build/target/rustc identity, declared host/filesystem/storage/cache metadata, B+ tree cache capacity, timestamp, and notes;
- `index.json` — format version, repository revision, protocol identifiers, and the exact archive file list.

The stable protocol identifiers are `execution_protocol = "fresh_counterbalanced_repeated_batch_v1"`, `attempt_protocol = "retain_all_requested_pairs_v1"`, and `format_version = 6`. Existing paths are never overwritten. Partial archive directories are removed if serialization or durable file creation fails.

Exploratory v6 intentionally omits `publication_admission` and `admission_protocol`. Supplying publication-only metadata while leaving admission at `exploratory` is rejected instead of creating an ambiguous archive.

## Publication format v7

`--admission publication-warm-v1` applies the same warm-only publication boundary used by the single-pair archive path to the complete repeated batch. Before either engine root or archive directory is created, the command requires:

1. a release build;
2. `--cache-state warm`;
3. a concrete Rust host target triple from `rustc -vV`;
4. non-empty `--host-label`, `--host-cpu`, `--host-memory`, and `--storage-device`;
5. non-empty `--filesystem` and `--mount-options`;
6. non-empty `--optimization-flags`, `--analysis-script-version`, and `--noise-budget`;
7. every publication metadata field to fit within 4 KiB after trimming; and
8. at least one requested pair to remain included after all explicit pre-run exclusions.

The v7 `publication_admission` object freezes `admission_protocol = "publication_warm_v1"`, `cache_policy = "trace_induced_warm"`, `durability_mode = "synced_single_operation"`, `pair_order_policy = "pair_seed_low_bit_then_alternate"`, the requested pair count, and exactly two ordered comparisons per included pair. `index.json` also records the admission protocol.

`cold_best_effort` is rejected in publication mode. The repository does not treat a string declaration as proof that operating-system, filesystem, controller, or device caches were evicted.

An admitted batch may still contain explicitly excluded pairs when their reasons were declared before engine creation, and those exclusions remain in the denominator. A batch with retained runtime/factory failures is still archived as v7 and then returns a non-zero process status.

Example shape:

```text
db-lab-batch \
  --trace mixed-42.json \
  --engine-root runs/mixed-42-engines \
  --archive-dir evidence/mixed-42-batch \
  --pair-seed 42 \
  --pairs 20 \
  --revision 0123456789abcdef0123456789abcdef01234567 \
  --admission publication-warm-v1 \
  --cache-state warm \
  --host-label perf-host-01 \
  --host-cpu "CPU model / pinned topology" \
  --host-memory "64 GiB / fixed channels" \
  --storage-device "NVMe model" \
  --filesystem ext4 \
  --mount-options "rw,noatime" \
  --optimization-flags "--release; RUSTFLAGS=-C target-cpu=native" \
  --analysis-script-version analysis@abc123 \
  --noise-budget host-noise-budget-v1
```

## Methodology boundary

Publication v7 proves that repository-side admission policy, metadata completeness, warm-cache labeling, repeated AB/BA ordering, and non-lossy attempt retention were enforced. It does not independently verify human-supplied hardware/mount labels, pin CPU affinity, disable turbo, control thermals/background load, or establish a pinned performance host by itself. Hosted CI remains correctness/build validation only.

Likewise, retaining a failed pair does not yet retain a duration/work sample for an individual failed internal REOPEN or compaction operation. That lower-level failure instrumentation remains a separate Phase 4 requirement.
