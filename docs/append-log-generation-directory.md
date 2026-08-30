# Append-log generation directory v2

This document defines the concrete, read-only generation-directory format used by the future append-log compaction switch. It implements the no-rollback selection law from `docs/append-log-generation-switch.md` and the marker-bound committed-prefix proof added by PR #59 without changing append-log file format v1.

The verifier command is:

```text
db-lab-log-generation-verify --directory data/log-generations
```

Its protocol identifier is `append_log_generation_directory_v2`.

This protocol defines **how retained generation evidence is read and validated**. It still does not implement the writer that durably publishes a marker, so a passing hosted-CI fixture is not evidence that parent-directory durability has been solved on a real filesystem.

## Strict directory namespace

A generation directory is a real directory, not a symlink. Its namespace is intentionally closed and permits only canonical pairs such as:

```text
generation-00000000000000000001.log
commit-00000000000000000001.marker
generation-00000000000000000002.log
commit-00000000000000000002.marker
...
```

Generation ids are decimal `u64` values greater than zero, encoded as exactly 20 digits with leading zeroes. Names that begin with a reserved prefix but are not exactly canonical fail verification. Any unrelated directory entry also fails verification.

At most 8,192 directory entries are scanned. This bound prevents an untrusted directory from turning verification into unbounded metadata work.

A canonical generation log without a matching marker is an **uncommitted generation**. It may be a complete compact image or a crash orphan; the verifier records its id but never opens it merely because its bytes look valid.

## Commit marker format v2

All integers are unsigned little-endian. The marker is exactly 64 bytes.

| Offset | Bytes | Meaning | v2 check |
| ---: | ---: | --- | --- |
| 0 | 8 | magic `DBLGCMT\0` | exact |
| 8 | 2 | marker format version | `2` |
| 10 | 2 | marker header length | `64` |
| 12 | 8 | generation id | nonzero and exactly matches filename id |
| 20 | 2 | referenced append-log format version | `1` |
| 22 | 2 | reserved | zero |
| 24 | 8 | committed-prefix byte length | at least the 16-byte append-log header |
| 32 | 4 | committed-prefix CRC-32 (IEEE) | recomputed over exactly the bound prefix |
| 36 | 4 | flags | zero |
| 40 | 8 | committed-prefix record count | must match prefix replay |
| 48 | 8 | committed-prefix next sequence | must match prefix replay and equal record count + 1 |
| 56 | 4 | secondary reserved field | zero |
| 60 | 4 | marker CRC-32 (IEEE) | CRC of marker bytes 0–59 |

CRC-32 is corruption detection only. It is not authentication, a signature, authorship evidence, or protection against a malicious collision construction.

Marker v1 bound only generation identity and append-log format. No writer ever durably published marker v1, and v1 cannot satisfy the PR #59 committed-prefix proof obligation. The v2 reader therefore intentionally rejects the old 32-byte marker shape instead of carrying forward an ambiguous recovery contract.

## What the committed-prefix proof means

A generation remains appendable after the switch, so the marker cannot bind the generation's eventual physical EOF. Instead it binds the exact complete compacted image that was verified before the switch. Later synchronized mutation records may extend the file beyond that prefix.

For the highest committed generation the reader first runs normal read-only `LogEngine::verify` over the current file, then independently re-verifies the marker-bound prefix:

1. the current source file must contain at least the marker-bound number of bytes;
2. exactly those bytes are streamed read-only from the source while CRC-32 is recomputed;
3. the prefix is materialized only into a temporary verification file outside the generation directory;
4. that exact prefix is passed to the canonical append-log v1 `LogEngine::verify` parser;
5. the prefix must be clean: `recoverable_tail = null`, `file_bytes = valid_bytes = committed_prefix.bytes`;
6. verified `record_count` and `next_sequence` must exactly equal the marker-bound values;
7. the full authoritative file must still have at least that many structurally valid bytes.

The temporary copy exists only to reuse the canonical parser without duplicating append-log decoding rules. Generation-directory verification never mutates, truncates, repairs, renames, or deletes the source generation or marker files.

This proves that a later recoverable tail starts **after** a retained complete marker-bound base prefix. It does not cryptographically prove who produced that prefix or that the future writer followed the required filesystem durability ordering; writer-side crash tests remain necessary.

## Selection algorithm

The verifier enumerates canonical names, then selects the **highest generation id for which a commit-marker filename exists**. It does not search for the highest valid-looking generation log.

Only the highest marker is authoritative for selection. The verifier then:

1. requires that highest marker path to be a real regular file rather than a symlink or non-file;
2. requires exactly 64 bytes and validates marker v2 completely;
3. requires the corresponding canonical generation log to exist as a real regular file;
4. runs read-only `LogEngine::verify` on that current log;
5. requires its append-log format version to equal the marker's referenced format version;
6. re-verifies the marker-bound committed prefix as described above;
7. if the current log has a recoverable tail, requires its `record_offset` to be at or after the end of the committed prefix.

If the highest marker is corrupt, its referenced generation log is missing/corrupt, or its committed-prefix proof fails, verification fails closed. It never falls back to a lower marker. This is the retained-artifact form of the no-rollback rule from `docs/append-log-generation-switch.md`.

Lower marker contents are not used to choose or validate the current authoritative generation. Damage to historical lower generations must not make a valid higher committed generation roll back.

## Recoverable final append

A canonical append-log v1 incomplete-final-append state is admissible only when the v2 marker proves a complete committed prefix before that tail.

A valid example is:

1. compacted base prefix is complete and marker-bound;
2. the generation becomes authoritative;
3. one or more later mutation records are appended successfully;
4. a subsequent mutation crashes partway through its final append.

The full verifier reports that final append as `recoverable_tail`, while the independent prefix verification remains clean. Generation-directory verification succeeds and remains read-only. A later mutable engine open may apply the existing v1 final-append repair rule.

By contrast, if the marker-bound prefix itself ends in a recoverable tail, verification fails closed even when both the prefix CRC and marker CRC match. That observable state is compatible with an invalid writer order in which the marker was published before the compacted base was complete.

## Verification output

Successful JSON reports:

- `protocol = "append_log_generation_directory_v2"`;
- marker format version;
- authoritative generation id and canonical log filename;
- highest generation id observed in any canonical log/marker name;
- all observed marker generation ids;
- generation-log ids that currently have no marker;
- the marker-bound `committed_prefix` proof;
- the independent `committed_prefix_verification` report;
- the complete current append-log `log_verification` report.

The uncommitted-id list is descriptive crash/orphan evidence. The verifier does not delete it.

## What remains before an authoritative switch exists

This reader does not create generation directories or markers and does not claim that a marker file written by an arbitrary process is durably published. The writer-side protocol still has to define and test:

- create-new generation allocation above every observed canonical id;
- construction, synchronization, and verification of a complete compact generation before marker publication;
- capture of the exact v2 committed-prefix byte length, CRC, record count, and next sequence from that verified image;
- marker create/write/checksum/file-sync mechanics;
- parent-directory entry durability on Linux, macOS, and Windows;
- exact crash injection before/during/after marker publication;
- routing subsequent mutations to the newly committed generation;
- safe old-generation and orphan cleanup only after the new marker is durably committed;
- migration or coexistence rules for the current legacy single-file `LogEngine` path.

Until writer-side publication and recovery are implemented against this v2 reader and the executable recovery model, the Phase 1 general compaction milestone remains open.
