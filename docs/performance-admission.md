# Performance publication admission

This document defines the first enforceable publication admission policy for Phase 4 performance evidence.
It does **not** claim that a portable user-space process can flush or independently prove the state of every
operating-system, filesystem, controller, or device cache.

## Why a separate admission policy exists

Exploratory experiment archives are useful for development, correctness inspection, and methodology work, but
an archive is not automatically publication-grade. The experimental constitution requires enough environment
metadata to reproduce and audit a performance claim, and it requires cache state to be identified rather than
silently assumed.

`db-lab experiment-archive-counterbalanced` therefore has two admission modes:

- `exploratory` (default): preserves the existing format-v2 success and format-v3 failed/excluded archives;
- `publication-warm-v1`: a strict release-only warm-cache protocol that emits format-v4 success evidence or
  format-v5 failed/excluded attempt evidence.

The publication protocol is intentionally warm-only. `cold_best_effort` remains valid descriptive metadata for
exploratory work, but it is rejected by `publication-warm-v1`; writing “cold” into JSON is not proof that kernel,
filesystem, controller, and device caches were actually evicted.

## `publication-warm-v1` admission rules

Before either engine is created, the CLI requires all of the following:

1. the binary is a release build (`cfg!(debug_assertions) == false`);
2. `--cache-state warm` is selected;
3. `rustc -vV` yields a concrete host target triple;
4. `--host-label`, `--host-cpu`, and `--host-memory` are present and non-empty;
5. `--storage-device` is present and non-empty;
6. `--filesystem` and `--mount-options` are present and non-empty;
7. `--optimization-flags` is present and non-empty;
8. `--analysis-script-version` is present and non-empty;
9. `--noise-budget` is present and non-empty;
10. all publication metadata fields are at most 4 KiB after trimming.

The protocol records the following fixed semantics rather than asking the caller to invent labels for them:

- `admission_protocol = "publication_warm_v1"`;
- `cache_policy = "trace_induced_warm"`;
- `durability_mode = "synced_single_operation"`;
- `repetition_count = 2` (one AB and one BA whole-run ordering).

`trace_induced_warm` means the engine state immediately preceding measured reopen/compaction samples was produced
by the same setup/measured trace and process under test. It makes no cold-cache claim and performs no privileged
cache flush. This is the only cache state the first portable publication protocol is willing to admit.

## Example

```text
db-lab experiment-archive-counterbalanced \
  --trace mixed-42.json \
  --first-btree-path btree-42-a.db \
  --first-lsm-path lsm-42-a \
  --second-btree-path btree-42-b.db \
  --second-lsm-path lsm-42-b \
  --pair-order left-then-right-first \
  --btree-cache-pages 64 \
  --revision 0123456789abcdef0123456789abcdef01234567 \
  --archive-dir evidence/mixed-42-publication \
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

A successful admitted run writes the existing raw counterbalanced payload plus a `publication_admission` object
in `environment.json`; `index.json` records the admission protocol. If the admitted invocation fails or is
explicitly excluded, the same admission record is retained in the format-v5 attempt archive, so the denominator
of attempted publication runs remains auditable.

## What the gate proves — and what it does not

The gate proves that the repository refused to label a run `publication_warm_v1` unless the required metadata,
release-build condition, warm-only cache policy, target triple, and AB/BA repetition protocol were present. It
does not independently verify that a human-supplied CPU/model/mount-options string is truthful, nor does it
pin CPU affinity, disable turbo, control thermals, or establish a stable noise budget by itself.

A real performance regression gate still requires a named pinned host and externally controlled operating
conditions as required by the experimental constitution. Hosted CI remains correctness/build validation only;
its timing must not be promoted into a performance baseline.
