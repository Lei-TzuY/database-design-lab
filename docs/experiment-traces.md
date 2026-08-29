# Shared Phase 4 experiment traces

Phase 4 compares B+ tree and LSM behavior only after both engines receive the exact same logical input.
`db-core` therefore owns a separate versioned `ExperimentTrace` schema instead of changing the Phase 1
correctness-workload format. Correctness workloads remain focused on PUT/GET/DELETE/REOPEN regression;
experiment traces add ordered range scans and an explicit setup/measurement boundary.

## Trace structure

`ExperimentTrace` v1 contains:

- `profile`: one stable experiment family;
- `seed`: the recorded SplitMix64 seed;
- `generator`: all stable-generator parameters;
- `setup_steps`: deterministic state-building operations excluded from amplification counters;
- `measured_steps`: the exact operations included in the measurement window.

The runner validates every key/value/range against the common contract, executes all setup steps, calls
`reset_amplification()` exactly once, and then executes every measured step in order. `REOPEN` may occur in
the measured window. B+ tree and LSM preserve their process-local measurement window across the common
same-handle reopen operation.

Trace format v1 accepts canonical generated traces only. Validation regenerates the declared seed/config
and requires the setup and measured vectors to match exactly; removing generator metadata or editing steps
without changing the metadata fails closed. A future hand-authored/custom family needs an explicit schema
and profile rather than borrowing a generated profile label.

Resource bounds are part of trace validity. A trace contains at most 1,000,000 total steps, 64 MiB of
combined encoded key/value payload, and range limits no greater than 1,000,000 rows. Generation checks a
conservative profile-specific payload upper bound before allocating step vectors. Runners also cap the
cumulative key/value payload produced by setup or measured outcomes at 64 MiB per phase; only the common
measured vector is retained in a report. These are defensive limits, not recommended experiment sizes.

Generated keys are fixed-width eight-byte **big-endian** unsigned ids. Numeric id order therefore equals
bytewise key order, making generated range boundaries architecture-independent. Values are fixed-size byte
strings filled by the specified SplitMix64 stream. The experiment generator has its own SplitMix64
implementation so future changes to the Phase 1 correctness generator cannot silently change Phase 4 traces.
Committed golden fingerprints cover every profile under one fixed configuration. Changing a generator rule
requires a trace-format revision and continued validation support for archived older traces.

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

Both candidates execute each setup action in lockstep and compare its complete logical outcome before moving
to the next action. Only after all setup outcomes agree are both instrumentation windows reset. Measured
actions are likewise executed and checked in lockstep; the first mismatch fails the experiment without
emitting amplification evidence. A successful `ExperimentComparisonReport` stores the full trace once, the
proven common measured-outcome vector once, and per-engine capabilities plus the exact common
`AmplificationReport`.
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

This runner establishes canonical bounded logical inputs, explicit setup/measurement boundaries, lockstep
setup/measured outcome equality, and shared structural amplification reporting. It does **not** establish a
fair latency benchmark by itself. Complete recovery-work accounting, failed/excluded samples, counterbalanced
engine order, a cache/filesystem protocol, and controlled-host pinning remain separate Phase 4 work.

## Evidence archives

`db-lab experiment-archive` is the publication boundary for raw Phase 4 evidence. It consumes one existing
trace, creates fresh B+ tree and LSM targets, runs the exact shared comparison, and then creates a new archive
directory containing four JSON files: `trace.json`, `comparison.json`, `environment.json`, and `index.json`.
The caller must provide `--revision`; the archive also records the db-lab package version, target OS/arch,
build profile, best-effort Rust compiler version, B+ tree cache capacity, a declared cache state, and optional
host/filesystem/storage labels. Existing archive paths are rejected and a failed multi-file write removes the
partial archive directory. Do not put credentials, serial numbers, or other secrets into labels/notes.

The manifest does not make timings comparable by itself. `cold_best_effort` is only a declaration, not proof
that kernel/device caches were flushed. Controlled-host pinning remains mandatory before latency claims or
regression thresholds.
