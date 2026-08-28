# Database Design Lab

Database Design Lab is a modular systems laboratory for testing how database architecture changes
when independently chosen design dimensions meet. It starts with one deliberately narrow question:
can two physical storage engines implement exactly the same binary key-value semantics, survive the
same recovery tests, and then be measured by the same methodology?

This repository explicitly **does not claim that there are exactly 750 database types**. Multiplying
six model labels, five storage labels, five concurrency labels, and five distribution labels happens
to produce 750 cells, but that arithmetic is not a taxonomy. The categories are not uniformly
exclusive, not equally implementable, and not fully orthogonal. A document model changes indexing
requirements; MVCC changes physical record layout and reclamation; replication changes when a write
may be acknowledged. The matrix is an experimental design space used to state hypotheses and expose
constraints—not a list of products we pretend to have built.

## Implemented baseline

The workspace currently contains six crates with executable behavior. The B+ tree is a complete common
persistent point/range engine; the LSM now has a WAL/MemTable plus crash-published SSTable/manifest stage:

| Crate | Implemented role |
| --- | --- |
| `db-core` | Binary KV semantics, explicit capabilities, versioned workload model, stable seeded generator, execution and differential harness |
| `db-storage-memory` | Deterministic in-memory reference/oracle engine |
| `db-storage-log` | Standalone, caller-serialized, checksummed append-only engine with tombstones, replay, reopen, inspection, verification, and incomplete-final-append recovery |
| `db-storage-btree` | Common persistent `KvEngine` with fixed 4 KiB checksummed pages, mirrored superblocks, COW `GET`/`PUT`/`DELETE`/`REOPEN`, half-open ordered `range_scan`, split/rebalance/root contraction, reachability-derived page reuse, overflow-backed 4 KiB keys and 1 MiB values, reachable-tree validation, and bounded validated-page caching |
| `db-storage-lsm` | Common persistent `KvEngine` with its own checksummed WAL, ordered MemTables, indexed/checksummed immutable SSTables, immutable manifest snapshots, mirrored `CURRENT` publication, WAL-tail replay, tombstones, and half-open range scans; no Bloom filters, WAL reclamation, levels, or compaction yet |
| `db-cli` | `db-lab generate`, `run`, `differential`, `verify`, and `inspect` |

The append log is the common persistent correctness foundation, not a disguised B+ tree or partial LSM.
The B+ tree uses a separate page file whose two mirrored superblocks define the committed physical
extent and published root. `PUT` and `DELETE` never overwrite a reachable page: they append replacement
leaves and ancestors, with insertion splitting overflowing pages and deletion merging or byte-balancing
an underfull child with an adjacent sibling before one final root publication. Deletion also contracts
a one-child root and can publish an empty tree after the last key. A crash before that root publication
therefore leaves the old root authoritative. Before each later mutation, committed pages outside the
current root's reachability are eligible for synchronized overwrite and reuse, so COW history becomes
allocation space without a persistent free-list. Keys that would overfill a tree cell and values
that cannot fit inline are stored in checksummed overflow-page chains; live key/value chains participate
in the same reachability/reuse proof. The B+ tree implements the common 4 KiB-key/1 MiB-value point
contract, including explicit `REOPEN`, and is differentially tested against the in-memory oracle.
Ordered B+ tree scans walk the internal hierarchy in key order rather than persisting leaf sibling
links, so scans introduce no extra COW/reuse edge. Deterministic mutation fault tests exercise appended
and recycled overflow/leaf/internal page writes, allocation metadata, and final root publication under
pre-write, torn-half-write, and post-sync error modes. Reopen must select either the complete old tree or
the complete new tree; only a fully synchronized root superblock whose caller still receives an I/O
error may expose the new state. Torn recycled orphans remain unreachable and are proven safely
overwriteable by a later mutation. Physical file compaction/truncation and arbitrary device/cache
power-loss modeling remain deferred.

The LSM is not an adapter around `db-storage-log`: it owns a distinct WAL format and keeps
sequence-tagged values/tombstones in an ordered mutable MemTable that freezes at a documented 64 KiB
resident estimate. A frozen table is synchronously encoded as an indexed/checksummed immutable SSTable,
then referenced by a checksummed immutable manifest snapshot, and only then published through the
inactive slot of an 8 KiB mirrored `CURRENT` file. Reopen validates the selected manifest/SSTables and
replays only WAL sequences above the manifest's durable watermark. The WAL deliberately retains its
complete history in this slice, and there are no Bloom filters, levels, or compaction, so this remains
correctness/recovery evidence—not yet a fair B+ tree performance comparison participant.

Current common semantics allow empty and arbitrary binary keys/values, cap keys at 4 KiB and values
at 1 MiB, distinguish missing values from empty values, and expose `PUT`, `GET`, `DELETE`, `REOPEN`,
and a bounded half-open ordered range API `[start, end)`, with `end = None` meaning unbounded. The
in-memory oracle, B+ tree, and LSM MemTables advertise ordered range support; the append log deliberately
does not, because its replay `BTreeMap` is not an on-disk ordered access path. Workload schema v1 still
serializes point/lifecycle steps only; reproducible generated range traces remain Phase 4 work. Transactions,
multi-process writers, LSM Bloom filters/levels/compaction/WAL reclamation, replication, SQL, MVCC,
Raft, graph, time-series, and columnar execution are not implemented.

## Run the laboratory

Install stable Rust, then:

```console
cargo test --workspace --locked
cargo run -p db-cli -- generate --seed 24301 --operations 1000 \
  --reopen-every 17 --output workload.json
cargo run -p db-cli -- differential --path experiment.db workload.json
cargo run -p db-cli -- differential --engine lsm --path experiment-lsm workload.json
cargo run -p db-cli -- verify experiment.db
cargo run -p db-cli -- inspect experiment.db --show-values
```

Run one engine and print every observable outcome:

```console
cargo run -p db-cli -- run --engine memory fixtures/workloads/semantics-v1.json
cargo run -p db-cli -- run --engine log --path lab.db \
  fixtures/workloads/semantics-v1.json
cargo run -p db-cli -- run --engine lsm --path lsm-dir \
  fixtures/workloads/semantics-v1.json
```

Workload byte strings are lowercase hexadecimal in JSON. Generated workloads record their seed and
schema version. `differential` refuses to reuse an existing persistent path so prior state cannot
silently contaminate a correctness comparison.

## Persistence and recovery contract

Append-log format v1 has a checksummed magic/version file header. Each record has its own magic,
version, kind, flags, monotonic sequence, bounded little-endian lengths, header checksum, and
full-record checksum. The decoder validates the physical extent and all limits before allocating
payload memory and uses checked arithmetic for every derived extent.

A successful append-log mutation has completed `write_all` and `sync_data` before the in-memory index
changes or the call returns. On reopen, every complete valid record is replayed. A structurally valid
but incomplete final append is discarded by truncating back to its starting offset and synchronizing
the repair. A checksum failure, absurd length, unknown record kind/version, sequence discontinuity, or
unrecognized tail fails closed. `verify` reports a recoverable partial tail without modifying it.

The LSM foundation stores its own WAL in an engine directory. Its header and each PUT/DELETE record
carry independent magic/version fields, bounded little-endian lengths, contiguous sequence numbers,
and header/full-record CRC-32 checksums. `write_all` and `sync_data` complete before a mutation enters
the MemTable or returns. Reopen replays complete records and truncates only a structurally canonical
incomplete final record; unknown directory entries, invalid headers, sequence gaps, absurd lengths,
unexplained tails, and complete checksum failures fail closed. No SSTable or manifest commit guarantee
is claimed because those formats do not exist yet.

The B+ tree pager uses a different commit unit. Two checksummed 4 KiB superblocks alternate metadata
generations. A newly allocated immutable page is synchronized before `page_count + 1` is written to
the inactive superblock and synchronized. Tree mutation is copy-on-write: the changed leaf and every
ancestor are appended as immutable pages. A key or value that requires overflow storage is first written as a tail-to-head chain of
checksummed overflow pages so every next-page target is already durable before its predecessor. The
replacement leaf/internal cell then stores the logical length and first overflow page id. Root/non-root tree
overflows split into new pages; deletion removes the target entry, merges sibling pages when their
combined encoded cells fit, otherwise
redistributes them by encoded byte size, and contracts a one-child root. Only after all replacement
pages are durable is the new root id published through another superblock transition. Reopen selects
the newest valid superblock and validates the complete reachable tree,
including sorted leaf keys, ordered separators, separator/child-minimum agreement, equal child heights,
non-overlapping ranges, cycle/duplicate-reference rejection, canonical key/value overflow-chain
lengths, and zero sibling pointers for the current point-tree representation. Overflow pages referenced
by live leaves or internal separators are part of the reachable set. Before a mutation, every committed page absent from that validated reachable
set is reusable. A recycled page is fully overwritten and `sync_data` completes before any new root can
reference it; no `page_count` transition is needed because its physical extent was already committed.
This reclaims logical allocation space but does not shrink the physical file.

These guarantees do not include multi-operation transactions, concurrent-process exclusion,
directory-entry durability for initial file creation, protection against lying storage hardware, or
cryptographic integrity. An I/O error during a persistent commit has an ambiguous outcome; the live
handle is poisoned and must be reopened. See [the append-log format](docs/on-disk-format.md),
[the B+ tree page format](docs/btree-page-format.md),
[the LSM WAL/MemTable format](docs/lsm-wal-format.md), and
[the experimental constitution](docs/experimental-constitution.md) for exact fault models.

## Experimental discipline

- [Experimental constitution](docs/experimental-constitution.md): semantics, correctness protocol,
  metrics, reproducibility, failure handling, and non-goals.
- [Design space](docs/design-space.md): definitions, capability/compatibility caveats, and coupling
  between dimensions.
- [Append-log format](docs/on-disk-format.md): exact append record bytes and recovery decisions.
- [B+ tree page format](docs/btree-page-format.md): mirrored superblocks, slotted pages, copy-on-write
  insert/delete publication, key/value overflow chains, split/rebalance behavior, validation, and current
  crash-state limits.
- [LSM WAL/MemTable foundation](docs/lsm-wal-format.md): directory/WAL bytes, replay and tail policy,
  deterministic MemTable freezing, and explicitly deferred SSTable/manifest behavior.
- [Roadmap](docs/roadmap.md): evidence-linked completed items and deliberately deferred phases.

GitHub Actions runs formatting, Clippy with warnings denied, tests, and rustdoc on stable Rust, checks
the declared Rust 1.85 minimum, and runs tests on Linux, macOS, and Windows. Persistent formats use
explicit endian conversion and no platform-specific filesystem API. Hosted CI timing is smoke evidence
only; this repository publishes no performance result and gates no performance regression without a
controlled host.

## License

MIT
