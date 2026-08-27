# Evidence-driven roadmap

Checkboxes are tied to implementation on the default branch. A phase title is not “complete” merely
because later work has begun.

## Phase 0 — Experimental constitution

- [x] Reject the “exactly 750 database types” framing and define the matrix as a constrained design
  space (`README.md`, `docs/design-space.md`).
- [x] Define current dimensions, capability status, compatibility caveats, and cross-dimension leakage
  (`docs/design-space.md`).
- [x] Define common semantics, correctness protocol, deterministic seed policy, minimized-regression
  rule, crash fault model, operational metrics, reproducibility requirements, and non-goals
  (`docs/experimental-constitution.md`).
- [x] Specify append-log v1 bytes and recovery decisions (`docs/on-disk-format.md`).

## Phase 1 — Common correctness laboratory

- [x] Versioned binary KV workload representation with `PUT`, `GET`, `DELETE`, and `REOPEN`
  (`db-core`).
- [x] Stable SplitMix64 deterministic generator with recorded seed/configuration (`db-core`).
- [x] Deterministic in-memory oracle (`db-storage-memory`).
- [x] Real persistent append-log engine with versioning, bounded decoding, checked arithmetic, header
  and record checksums, replay, synced mutations, tombstones, and explicit tail recovery
  (`db-storage-log`).
- [x] Step-by-step differential harness and deterministic recorded-seed state-machine tests.
- [x] Adversarial tests for overwrite/delete/reinsert, reopen boundaries, binary/empty/boundary values,
  prefix interruption, checksum failure, absurd lengths, offset overflow, duplicate/tombstone replay,
  and pre-existing truncated headers.
- [x] CLI generation, execution, differential comparison, non-mutating verification, and inspection.
- [x] Stable-Rust formatting, Clippy, tests, docs, and cross-platform CI.
- [ ] Automated shrinking/minimization command for newly discovered failing traces. Until implemented,
  minimization is manual and the constitution still requires a committed deterministic regression.
- [ ] Compaction. This remains deliberately absent from the append foundation.

## Phase 2 — B+ tree engine

Do not create a B+ tree crate until its first PR includes executable pager/page behavior.

- [x] Write page-format and crash-state design with version/checksum/free-space invariants
  (`docs/btree-page-format.md`).
- [x] Implement bounded pager and cache with corruption validation (`db-storage-btree`): mirrored
  superblocks, synchronized immutable page allocation, slotted-page packing, root metadata commits,
  checksum/reference validation, bounded cache eviction, interrupted-allocation recovery, and torn
  superblock/truncation/corruption tests.
- [ ] Implement lookup/insert and root/non-root split properties.
- [ ] Implement deletion, redistribution/merge, root contraction, and space reuse.
- [ ] Expose true ordered scans through the common capability contract.
- [ ] Add reopen, torn-page/update fault injection, differential state machines, and deterministic
  regressions before performance work.

## Phase 3 — LSM engine

Begin only after the B+ tree and common ordered semantics are trustworthy.

- [ ] Specify WAL/SSTable/manifest formats and atomic version-set transitions.
- [ ] Implement WAL recovery and mutable/immutable MemTables.
- [ ] Implement immutable sorted tables with indexes and checksums.
- [ ] Add Bloom filters with measured false-positive configuration.
- [ ] Implement tombstone-aware reads, levels, compaction, and crash-safe manifest recovery.
- [ ] Add compaction fault injection, deterministic differential tests, and instrumentation validation.

## Phase 4 — Fair B+ tree versus LSM experiments

- [ ] Freeze common point/range/write/delete/durability semantics and capability preflight.
- [ ] Implement reproducible point-read, range-scan, sequential-write, random-write, and mixed traces.
- [ ] Validate counters for read, write, and space amplification on hand-computable traces.
- [ ] Measure recovery cost and compaction stall distributions.
- [ ] Archive raw data and environment manifests; publish no result before this evidence exists.
- [ ] Establish a controlled pinned performance host before adding regression gates.

## Phase 5+ — selected extensions

Choose one hypothesis at a time based on evidence from the storage laboratory. Possible work includes a
relational logical layer, explicit transaction semantics and one concurrency protocol, or standalone
state-machine integration for replication research. Raft, MVCC, SQL, graph, time-series, and columnar
engines remain out of scope until selected by a focused design proposal with a real implementation
plan and comparable correctness oracle.
