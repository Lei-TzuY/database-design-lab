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
- [x] Implement binary point lookup/insertion plus root and non-root split propagation using immutable
  copy-on-write path replacement and atomic root publication. Reopen validates the complete reachable
  tree and tests cover binary/empty keys, overwrite, multi-level splits, reopen, input bounds, and
  unpublished shadow-page behavior.
- [x] Implement copy-on-write deletion with missing-key no-op semantics, byte-aware adjacent-sibling
  redistribution/merge, empty-tree publication, and root contraction; deterministic multi-level tests
  delete a height-3+ tree down to one leaf and validate reopen.
- [x] Reclaim unreachable copy-on-write history as reusable allocation space by deriving orphan page
  ids from validated current-root reachability before each mutation. Recycled pages are synchronized
  before root publication, never overwrite the authoritative tree, and deterministic tests prove file
  page count stabilizes across repeated updates and empty-tree reuse. Physical file compaction remains deferred.
- [x] Support values through the common 1 MiB limit with checksummed overflow-page chains. Leaf
  references preserve logical length, overflow pages are committed tail-to-head before leaf/root
  publication, reopen validates canonical chains, and deterministic tests cover 1 MiB round-trip,
  delete, and orphan-page reuse without unbounded page-count growth.
- [x] Expose true ordered scans through the common capability contract using bounded half-open
  `[start, end)` traversal over internal child order rather than leaf sibling links. Exact child minima
  prune pre-start subtrees and stop at the upper bound; tests prove sorted/limited/read-only behavior,
  reopen/delete/overflow-value correctness, and equality with the in-memory oracle.
- [x] Admit B+ tree to the common `KvEngine` differential harness. Keys through 4 KiB use inline
  descriptors or checksummed overflow blobs in leaves and internal exact-minimum separators; trait-level
  capabilities/reopen match the common contract, and deterministic differential tests cover empty/binary
  keys, the 4 KiB/1 MiB size limits, overwrite, delete, and repeated reopen against the memory oracle.
- [x] Add deterministic mutation fault injection beyond the unpublished-shadow-page protocol. The
  pager records durable write classes and tests pre-write, synchronized half-write, and post-sync
  reported failures across appended/recycled overflow, leaf, and internal pages plus allocation/root
  superblocks. Reopen is constrained to the complete old or complete new tree; tests also cover final
  root clearing and prove torn recycled orphans can be overwritten safely by a later mutation.

## Phase 3 — LSM engine

Begin only after the B+ tree and common ordered semantics are trustworthy.

- [x] Specify and implement WAL/SSTable/manifest persistence through atomic flush installation. WAL v1
  remains in `docs/lsm-wal-format.md`; `docs/lsm-sstable-manifest-format.md` defines indexed/checksummed
  immutable SSTables, complete immutable manifest snapshots, mirrored 4 KiB `CURRENT` slots, durable
  sequence watermarks, canonical orphan handling, and the WAL-backed interrupted-install recovery rule.
- [ ] Specify and implement WAL segment rotation/reclamation. The current single WAL intentionally retains
  complete history so reclamation does not share the first SSTable/manifest publication protocol.
- [x] Implement an independent versioned/checksummed WAL with synchronized PUT/tombstone records,
  fail-closed bounded replay, canonical incomplete-tail recovery, and ordered mutable/immutable
  MemTables. Fixed-seed differential tests cover reopen after every operation; deterministic tests
  cover freeze/read precedence, ordered ranges, 4 KiB/1 MiB bounds, structural WAL prefixes, bit flips,
  absurd lengths, sequence gaps, tombstones, and undeclared directory entries.
- [x] Implement immutable sorted tables with complete indexes, per-record/index/header/footer checksums,
  whole-file checksum validation, full 4 KiB-key/1 MiB-value support, and manifest-bound key/extent metadata.
- [ ] Add Bloom filters with measured false-positive configuration.
- [x] Implement tombstone-aware multi-SSTable point/range reads plus crash-safe flush manifest recovery.
  Tests cover authoritative SSTable/manifest corruption, latest-CURRENT-slot fallback with WAL replay,
  unreferenced canonical orphans, mutable WAL tails, and maximum-value flush/reopen.
- [ ] Add levels, overlap policy, compaction, obsolete-file deletion, and safe tombstone dropping.
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
