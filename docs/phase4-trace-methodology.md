# Phase 4 reproducible trace methodology

Phase 4 compares the implemented B+ tree and LSM engines only after they pass the same logical trace.
This document defines the first shared trace generator and the evidence boundary it supports. It does
not define a stable timing benchmark and it does not turn architecture-specific structural counters
into device I/O measurements.

## Evidence boundary

Every generated trace is versioned and records:

- trace format version;
- generator revision;
- workload profile;
- SplitMix64 seed;
- measured operation count;
- key-space cardinality;
- exact generated value length;
- range width/result limit; and
- optional measured-operation reopen cadence.

The trace is split into `setup` and `measured` phases. Setup establishes identical state on both engines
and is intentionally excluded from the process-local amplification window. The harness executes setup
step by step on both engines, refuses any logical mismatch, resets both amplification counters, and only
then executes the measured phase. A `REOPEN` inside the measured phase remains part of the experiment
lifecycle and does not reset the instrumentation window.

Before any trace executes, `validate_experiment_compatibility` requires the two engines to agree on the
logical model, caller/concurrency contract, persistence class, distribution mode, ordered-range
capability, and key/value limits. Storage architecture and crash-recovery mechanism are intentionally
allowed to differ because they are the independent variables.

## Stable generated data

Generator revision 1 uses the repository's specified SplitMix64 implementation rather than a third-party
RNG API. Every logical key is the eight-byte big-endian encoding of its integer key id. This gives one
portable, fixed-width bytewise ordering and avoids accidental lexical ordering differences between
variable-width decimal strings.

PUT values have one exact configured length. Their bytes are generated from a separate deterministic
SplitMix64 stream derived from the trace seed, key id, and mutation revision. This keeps payload bytes
repeatable without making the operation-selection stream depend on value length.

Changing any generator rule requires a new generator revision. Existing archived evidence must continue
to identify the revision that produced it.

## Profiles

### `point-read`

Setup PUTs every key in `[0, key_space)` and reopens both engines. The measured phase performs seeded
point GETs over that same key domain. This measures successful logical point reads over identical state.

### `range-scan`

Setup PUTs every key in `[0, key_space)` and reopens both engines. Each measured scan chooses a seeded
start key, constructs a bounded half-open `[start, end)` interval, and uses `range_width` as both the
interval width and result limit. The final interval may be shorter at the top of the key domain.

### `sequential-write`

There is no setup state. The measured phase issues PUTs over a cyclic ascending key sequence. Operation
index is part of the deterministic value stream, so overwriting a wrapped key still changes its payload.

### `random-write`

There is no setup state. Each measured PUT selects its key from the configured key domain with the
profile-specific seeded stream. Operation index again contributes to the deterministic value payload.

### `mixed`

Setup PUTs every key and reopens both engines. The measured selector is fixed at:

- 30% PUT;
- 30% GET;
- 15% DELETE; and
- 25% bounded range scan.

Deletes can make later GETs miss and ranges shorter; those outcomes are part of the common logical trace,
not normalized away.

## Logical equality gate

For every setup and measured action the harness executes the same encoded step against the B+ tree and
LSM engines and compares the complete logical outcome:

- previous value returned by PUT;
- value returned by GET;
- previous value returned by DELETE;
- ordered key/value rows returned by range scan; or
- successful REOPEN.

The first mismatch fails the experiment. Amplification reports are emitted only after both engines have
agreed on every measured outcome. The evidence therefore cannot silently compare performance counters
for different logical database histories.

To make archived evidence easy to identify, the report includes FNV-1a fingerprints of the exact JSON
trace and of the framed setup/measured outcome streams. These fingerprints are reproducibility labels,
not cryptographic integrity claims.

## Amplification evidence

The B+ tree and LSM engines both return `db_core::AmplificationReport`, but the read numerator retains an
explicit architecture-specific unit:

- B+ tree point/range reads: validated `btree_page_access` events, including cache hits;
- LSM point reads: `lsm_sstable_consult` events; and
- LSM range reads: `lsm_sstable_version_decoded` events.

Those units are not interchangeable physical-I/O events. The shared runner is useful because it freezes
the logical input and report shape, not because it makes unlike internals equivalent.

The byte-based write and primary-structure ratios likewise retain each engine's documented accounting
boundary from `docs/amplification-methodology.md`. The Phase 4 evidence files do not measure filesystem
metadata, page-cache misses, syscall counts, device read/write bytes, fsync latency, compaction latency,
or wall-clock throughput.

## CLI and CI evidence

A fresh comparison can be produced with:

```console
cargo run --locked -p db-cli -- experiment \
  --profile mixed \
  --seed 15111065706836454659 \
  --operations 256 \
  --key-space 512 \
  --value-bytes 512 \
  --range-width 32 \
  --reopen-every 64 \
  --btree-path target/mixed.btree \
  --lsm-path target/mixed.lsm \
  --output target/mixed-evidence.json
```

Both persistent paths must be new. Reusing old state is deliberately rejected by the engine constructors.

CI runs all five profiles from a fixed seed/configuration, archives the raw JSON reports plus a small
environment manifest, and treats the run as correctness/reproducibility evidence only. GitHub-hosted
runner timings are not recorded as performance results or used as regression gates.

## Deferred evidence

This slice does not yet provide:

- controlled-host latency or throughput comparison;
- device-level read/write attribution;
- recovery-time distributions;
- compaction-stall distributions;
- warm/cold operating-system page-cache protocols; or
- a pinned performance regression host.

Those require separate methodology and remain explicit Phase 4 roadmap work.
