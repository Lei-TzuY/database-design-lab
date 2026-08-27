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

The first baseline contains four crates, all with executable behavior:

| Crate | Implemented role |
| --- | --- |
| `db-core` | Binary KV semantics, explicit capabilities, versioned workload model, stable seeded generator, execution and differential harness |
| `db-storage-memory` | Deterministic in-memory reference/oracle engine |
| `db-storage-log` | Standalone, caller-serialized, checksummed append-only engine with tombstones, replay, reopen, inspection, verification, and incomplete-final-append recovery |
| `db-cli` | `db-lab generate`, `run`, `differential`, `verify`, and `inspect` |

The append log is a foundation, not a disguised B+ tree or partial LSM. It is the smallest persistent
architecture that makes framing, versioning, checksums, bounded decoding, durability boundaries,
replay, corruption policy, and differential testing real before more complex engines are attempted.

Current common semantics allow empty and arbitrary binary keys/values, cap keys at 4 KiB and values
at 1 MiB, distinguish missing values from empty values, and expose `PUT`, `GET`, `DELETE`, and an
explicit `REOPEN` workload boundary. Range scans, transactions, multi-process writers, compaction,
replication, SQL, MVCC, Raft, graph, time-series, and columnar execution are not implemented.

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

Format v1 has a checksummed magic/version file header. Each record has its own magic, version, kind,
flags, monotonic sequence, bounded little-endian lengths, header checksum, and full-record checksum.
The decoder validates the physical extent and all limits before allocating payload memory and uses
checked arithmetic for every derived extent.

A successful mutation has completed `write_all` and `sync_data` before the in-memory index changes or
the call returns. On reopen, every complete valid record is replayed. A structurally valid but
incomplete final append is discarded by truncating back to its starting offset and synchronizing the
repair. A checksum failure, absurd length, unknown record kind/version, sequence discontinuity, or
unrecognized tail fails closed. `verify` reports a recoverable partial tail without modifying it.

These guarantees do not include multi-operation transactions, concurrent-process exclusion,
directory-entry durability for initial file creation, protection against lying storage hardware, or
cryptographic integrity. An I/O error during append has an ambiguous commit outcome; the live engine
is poisoned and must be reopened. See [the on-disk format](docs/on-disk-format.md) for exact bytes and
[the experimental constitution](docs/experimental-constitution.md) for the fault model.

## Experimental discipline

- [Experimental constitution](docs/experimental-constitution.md): semantics, correctness protocol,
  metrics, reproducibility, failure handling, and non-goals.
- [Design space](docs/design-space.md): definitions, capability/compatibility caveats, and coupling
  between dimensions.
- [Roadmap](docs/roadmap.md): evidence-linked completed items and deliberately deferred phases.

GitHub Actions runs formatting, Clippy with warnings denied, tests, and rustdoc on stable Rust, checks
the declared Rust 1.85 minimum, and runs tests on Linux, macOS, and Windows. Format parsing uses
explicit endian conversion and no platform-specific filesystem API. Hosted CI timing is smoke
evidence only; this repository publishes no performance result and gates no performance regression
without a controlled host.

## License

MIT
