# Append-log generation directory v1

This document defines the first concrete, read-only generation-directory format for append-log compaction recovery. It implements the selection law frozen in `docs/append-log-generation-switch.md` without changing append-log file format v1.

The verifier command is:

```text
db-lab-log-generation-verify --directory data/log-generations
```

Its protocol identifier is `append_log_generation_directory_v1`.

This PR defines **how retained generation evidence is read and validated**. It does not yet implement the writer that makes a commit marker durable, so a passing hosted-CI fixture is not evidence that directory-entry durability has been solved on a real filesystem.

## Strict directory namespace

A generation directory is a real directory, not a symlink. Its namespace is intentionally closed and currently permits only:

```text
generation-00000000000000000001.log
commit-00000000000000000001.marker
generation-00000000000000000002.log
commit-00000000000000000002.marker
...
```

Generation ids are decimal `u64` values greater than zero, encoded as exactly 20 digits with leading zeroes. Names that begin with a reserved prefix but are not exactly canonical fail verification. Any unrelated directory entry also fails verification.

At most 8,192 directory entries are scanned by v1. This bound prevents an untrusted directory from turning verification into unbounded metadata work.

A canonical generation log without a matching marker is an **uncommitted generation**. It may be a complete compact image or a crash orphan; the verifier records its id but never opens it merely because its bytes look valid.

## Commit marker format v1

All integers are unsigned little-endian. The marker is exactly 32 bytes.

| Offset | Bytes | Meaning | v1 check |
| ---: | ---: | --- | --- |
| 0 | 8 | magic `DBLGCMT\0` | exact |
| 8 | 2 | marker format version | `1` |
| 10 | 2 | marker header length | `32` |
| 12 | 8 | generation id | nonzero and exactly matches filename id |
| 20 | 2 | referenced append-log format version | `1` |
| 22 | 2 | reserved | zero |
| 24 | 4 | flags | zero |
| 28 | 4 | CRC-32 (IEEE) | CRC of bytes 0–27 |

CRC-32 is corruption detection only. It is not authentication, a signature, authorship evidence, or protection against a malicious collision construction.

The marker binds the generation id and append-log format family. It deliberately does not bind the generation file byte length because an authoritative generation must remain appendable after the switch; later synchronized mutations legitimately change its physical length.

## Selection algorithm

The verifier enumerates canonical names, then selects the **highest generation id for which a commit-marker filename exists**. It does not search for the highest valid-looking generation log.

Only the highest marker is authoritative for selection. The verifier then:

1. requires that highest marker path to be a real regular file rather than a symlink or non-file;
2. requires exactly 32 bytes;
3. validates magic, CRC, marker version, header length, generation-id/filename binding, append-log format version, reserved field, and flags;
4. requires the corresponding canonical generation log to exist as a real regular file;
5. runs read-only `LogEngine::verify` on that log;
6. requires the verified append-log format version to equal the marker's referenced format version.

If the highest marker is corrupt or its referenced generation log is missing/corrupt, verification fails closed. It never falls back to a lower marker. This is the concrete retained-artifact form of the no-rollback rule from `docs/append-log-generation-switch.md`.

Lower marker contents are not used to choose or validate the current authoritative generation. Damage to historical lower generations must not make a valid higher committed generation roll back.

## Recoverable final append

If the authoritative generation log has the canonical append-log v1 incomplete-final-append state, `LogEngine::verify` reports it as `recoverable_tail` and generation-directory verification still succeeds.

The generation verifier is read-only: it does not call mutable `LogEngine::open`, truncate the tail, or synchronize a repair. A later engine open may apply the already-defined v1 tail-recovery rule to the selected authoritative generation.

## Verification output

Successful JSON reports:

- `protocol = "append_log_generation_directory_v1"`;
- marker format version;
- authoritative generation id and canonical log filename;
- highest generation id observed in any canonical log/marker name;
- all observed marker generation ids;
- generation-log ids that currently have no marker;
- the complete existing append-log `VerificationReport` for the authoritative log.

The uncommitted-id list is descriptive crash/orphan evidence. The verifier does not delete it.

## What remains before an authoritative switch exists

This reader does not create generation directories or markers and does not claim that a marker file written by an arbitrary process is durably published. The next writer-side protocol still has to define and test:

- create-new generation allocation above every observed canonical id;
- construction and verification of a complete compact generation before marker publication;
- marker create/write/checksum/sync mechanics;
- parent-directory durability on Linux, macOS, and Windows;
- exact crash injection before/during/after marker publication;
- routing subsequent mutations to the newly committed generation;
- safe old-generation and orphan cleanup after the new marker is durable;
- migration or coexistence rules for the current legacy single-file `LogEngine` path.

Until writer-side publication and recovery are implemented against this reader and the #57 model, the Phase 1 general compaction milestone remains open.