# Shared Phase 4 experiment traces

Phase 4 compares B+ tree and LSM behavior only after both engines receive the exact same logical input.
`db-core` therefore owns a separate versioned `ExperimentTrace` schema instead of changing the Phase 1
correctness-workload format. Correctness workloads remain focused on PUT/GET/DELETE/REOPEN regression;
experiment traces add ordered range scans and an explicit setup/measurement boundary.

## Trace structure

`ExperimentTrace` v1 contains:

- `profile`: one stable experiment family;
- `seed`: the recorded SplitMix64 seed for generated traces;
- `generator`: all generator parameters when the trace is generated rather than hand-authored;
- `setup_steps`: deterministic state-building operations excluded from amplification counters;
- `measured_steps`: the exact operations included in the measurement window.

The runner validates every key/value/range against the common contract, executes all setup steps, calls
`reset_amplification()` exactly once, and then executes every measured step in order. `REOPEN` may occur in
the measured window. B+ tree and LSM preserve their process-local measurement window across the common
same-handle reopen operation.

Generated keys are fixed-width eight-byte **big-endian** unsigned ids. Numeric id order therefore equals
bytewise key order, making generated range boundaries architecture-independent. Values are fixed-size byte
strings filled by the specified SplitMix64 stream. The experiment generator has its own SplitMix64
implementation so future changes to the Phase 1 correctness generator cannot silently change Phase 4 traces.

## Stable profiles

### `point_read`

Setup writes every id in `[0, key_space)`. Measured operations are GET only, plus optional REOPEN steps.
Each GET is deterministically selected as 80% hit / 20% miss. Hits read ids in the seeded key space; misses
read ids in `[key_space, 2 * key_space)`. Setup writes are outside the amplification window.

### `range_scan`

Setup writes every id in `[0, key_space)`. Every measured operation selects one start id uniformly from the
key space and requests `[start, min(start + range_limit, key_space))` with `limit = range_limit`. The generator
requires `0 < range_limit <= key_space`.

### `sequential_write`

Setup is empty. Measured operations PUT distinct ids `0, 1, ... operations - 1` in ascending bytewise order.
The generator requires `key_space >= operations`; this prevents the sequential profile from silently turning
into an overwrite workload.

### `random_write`

Setup is empty. Every measured operation PUTs one uniformly selected id from `[0, key_space)`. Repeated ids
are intentional and expose overwrite/update behavior under a stable random stream.

### `mixed`

Setup writes every even id in `[0, key_space)`, leaving odd ids absent so the measured phase contains both
existing and missing keys. Each measured logical operation uses this fixed distribution:

- 40% PUT;
- 30% GET;
- 15% RANGE_SCAN;
- 15% DELETE.

PUT/GET/DELETE ids are uniformly selected from the key space. Range generation matches the `range_scan`
profile. Optional REOPEN steps are inserted after every configured number of measured logical operations and
do not change the configured logical-operation count.

## Comparison runner

`compare_experiment_trace` first applies `validate_experiment_compatibility`. Logical model,
caller-serialization contract, persistence class, standalone distribution, ordered-range support, and key/value
limits must match. Storage architecture and crash-recovery mechanism are deliberately allowed to differ.

Both candidates then execute the same setup and measured vectors. The comparison is rejected if any measured
logical outcome differs. A successful `ExperimentComparisonReport` stores the full trace once, the proven
common outcome vector once, and per-engine capabilities plus the exact common `AmplificationReport`.
Read-work units remain architecture-specific (`btree_page_access`, `lsm_sstable_consult`, and
`lsm_sstable_version_decoded`) and must not be interpreted as interchangeable device I/O.

## CLI

Generate a trace:

```text
db-lab experiment-generate \
  --profile mixed \
  --seed 42 \
  --operations 1000 \
  --key-space 4096 \
  --value-bytes 128 \
  --range-limit 16 \
  --reopen-every 250 \
  --output mixed-42.json
```

Run it against fresh candidates and write a self-contained report:

```text
db-lab experiment-compare \
  --trace mixed-42.json \
  --btree-path btree-42.db \
  --lsm-path lsm-42 \
  --btree-cache-pages 64 \
  --output mixed-42-report.json
```

The three output/storage paths must be distinct and must not already exist. This prevents an experiment from
silently inheriting old engine state or overwriting prior evidence.

## Scope boundary

This runner establishes identical logical inputs, explicit setup/measurement boundaries, exact logical outcome
equality, and shared structural amplification reporting. It does **not** establish a fair latency benchmark by
itself. Controlled-host CPU/device/filesystem settings, cache-state protocol, latency distributions, recovery
cost, compaction stalls, and archived environment manifests remain separate Phase 4 work.
