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

The workspace currently contains five crates with executable behavior. The B+ tree crate now includes
an executable persistent point-operation slice, but is not yet a complete common `KvEngine`:

| Crate | Implemented role |
| --- | --- |
| `db-core` | Binary KV semantics, explicit capabilities, versioned workload model, stable seeded generator, execution and differential harness |
| `db-storage-memory` | Deterministic in-memory reference/oracle engine |
| `db-storage-log` | Standalone, caller-serialized, checksummed append-only engine with tombstones, replay, reopen, inspection, verification, and incomplete-final-append recovery |
| `db-storage-btree` | Fixed 4 KiB checksummed pages and mirrored superblocks plus copy-on-write binary `GET`/`PUT`/`DELETE`, root/non-root splits, byte-aware delete redistribution/merge, root contraction, reachable-tree validation, and bounded validated-page caching |
| `db-cli` | `db-lab generate`, `run`, `differential`, `verify`, and `inspect` |

The append log is the common persistent correctness foundation, not a disguised B+ tree or partial LSM.
The B+ tree uses a separate page file whose two mirrored superblocks define the committed physical
extent and published root. `PUT` and `DELETE` never overwrite a reachable page: they append replacement
leaves and ancestors, with insertion splitting overflowing pages and deletion merging or byte-balancing
an underfull child with an adjacent sibling before one final root publication. Deletion also contracts
a one-child root and can publish an empty tree after the last key. A crash before that root publication
therefore leaves the old root authoritative; committed shadow pages are unreachable history. Space
reclamation/reuse, ordered scans, large overflow values, and a common `KvEngine` implementation remain deferred.

Current common semantics allow empty and arbitrary binary keys/values, cap keys at 4 KiB and values
at 1 MiB, distinguish missing values from empty values, and expose `PUT`, `GET`, `DELETE`, and an
explicit `REOPEN` workload boundary. The current B+ tree point slice intentionally has a narrower
page-local key/value bound and therefore is not yet admitted to common differential experiments.
Range scans, transactions, multi-process writers, compaction, replication, SQL, MVCC, Raft, graph,
time-series, and columnar execution are not implemented.

## Run the laboratory

Install stable Rust, then:

```console
cargo test --workspace --locked
cargo run -p db-cli -- generate --seed 24301 --operations 1000 \
  --reopen-every 17 --output workload.json
cargo run -p db-cli -- differential --path experiment.db workload.json
cargo run -p db-cli -- verify experiment.db
cargo run -p db-cli -- inspect experiment.db --show-values
```

Run one engine and print every observable outcome:

```console
cargo run -p db-cli -- run --engine memory fixtures/workloads/semantics-v1.json
cargo run -p db-cli -- run --engine log --path lab.db \
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

The B+ tree pager uses a different commit unit. Two checksummed 4 KiB superblocks alternate metadata
generations. A newly allocated immutable page is synchronized before `page_count + 1` is written to
the inactive superblock and synchronized. Tree mutation is copy-on-write: the changed leaf and every
ancestor are appended as immutable pages. Root/non-root overflows split into new pages; deletion
removes the target entry, merges sibling pages when their combined encoded cells fit, otherwise
redistributes them by encoded byte size, and contracts a one-child root. Only after all replacement
pages are durable is the new root id published through another superblock transition. Reopen selects
the newest valid superblock and validates the complete reachable tree,
including sorted leaf keys, ordered separators, separator/child-minimum agreement, equal child heights,
non-overlapping ranges, and cycle/duplicate-reference rejection. Unreachable historical pages are not
reclaimed yet.

These guarantees do not include multi-operation transactions, concurrent-process exclusion,
directory-entry durability for initial file creation, protection against lying storage hardware, or
cryptographic integrity. An I/O error during a persistent commit has an ambiguous outcome; the live
handle is poisoned and must be reopened. See [the append-log format](docs/on-disk-format.md),
[the B+ tree page format](docs/btree-page-format.md), and
[the experimental constitution](docs/experimental-constitution.md) for exact fault models.

## Experimental discipline

- [Experimental constitution](docs/experimental-constitution.md): semantics, correctness protocol,
  metrics, reproducibility, failure handling, and non-goals.
- [Design space](docs/design-space.md): definitions, capability/compatibility caveats, and coupling
  between dimensions.
- [Append-log format](docs/on-disk-format.md): exact append record bytes and recovery decisions.
- [B+ tree page format](docs/btree-page-format.md): mirrored superblocks, slotted pages, copy-on-write
  insert/delete publication, split/rebalance behavior, validation, and current crash-state limits.
- [Roadmap](docs/roadmap.md): evidence-linked completed items and deliberately deferred phases.

GitHub Actions runs formatting, Clippy with warnings denied, tests, and rustdoc on stable Rust, checks
the declared Rust 1.85 minimum, and runs tests on Linux, macOS, and Windows. Persistent formats use
explicit endian conversion and no platform-specific filesystem API. Hosted CI timing is smoke evidence
only; this repository publishes no performance result and gates no performance regression without a
controlled host.

## License

MIT
