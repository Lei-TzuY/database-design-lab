# Append-log generation directory v2

This document defines the concrete generation-directory format used by the append-log compaction switch work. It implements the no-rollback selection law from `docs/append-log-generation-switch.md` and the marker-bound committed-prefix proof added by PR #59 without changing append-log file format v1.

The verifier command is:

```text
db-lab-log-generation-verify --directory data/log-generations
```

Its protocol identifier is `append_log_generation_directory_v2`.

The reader defines how retained generation evidence is selected and validated. `docs/append-log-generation-publication.md` defines the first writer-side marker publication primitive: durable publication on supported Unix targets. Mutation routing, old-generation cleanup, Windows-equivalent publication, and the complete compaction switch remain unfinished.

## Strict directory namespace

A generation directory is a real directory, not a symlink. Its namespace is intentionally closed and permits only canonical names of these forms:

```text
generation-00000000000000000001.log
commit-00000000000000000001.marker
staging-commit-00000000000000000002.marker
generation-00000000000000000002.log
...
```

Generation ids are decimal `u64` values greater than zero, encoded as exactly 20 digits with leading zeroes. Names that begin with a reserved prefix but are not exactly canonical fail verification. Any unrelated directory entry also fails verification.

At most 8,192 directory entries are scanned. This bound prevents an untrusted directory from turning verification into unbounded metadata work.

A canonical generation log without a matching final marker is an **uncommitted generation**. It may be a complete compact image or a crash orphan; the verifier records its id but never opens it merely because its bytes look valid.

A canonical `staging-commit-%020d.marker` is **non-authoritative publication residue**. Its bytes are not decoded or trusted for selection. The reader records its generation id for observability and otherwise ignores it. This allows a crash before final marker publication to leave a staging name without blocking recovery. Only final `commit-%020d.marker` names carry commit authority.

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

This proves that a later recoverable tail starts **after** a retained complete marker-bound base prefix. It does not cryptographically prove who produced that prefix. Writer-side ordering is a separate property; the Unix publisher establishes an explicit synchronization order documented in `docs/append-log-generation-publication.md`.

## Selection algorithm

The verifier enumerates canonical names, then selects the **highest generation id for which a final commit-marker filename exists**. It does not search for the highest valid-looking generation log and never promotes a staging marker.

Only the highest final marker is authoritative for selection. The verifier then:

1. requires that highest marker path to be a real regular file rather than a symlink or non-file;
2. requires exactly 64 bytes and validates marker v2 completely;
3. requires the corresponding canonical generation log to exist as a real regular file;
4. runs read-only `LogEngine::verify` on that current log;
5. requires its append-log format version to equal the marker's referenced format version;
6. re-verifies the marker-bound committed prefix as described above;
7. if the current log has a recoverable tail, requires its `record_offset` to be at or after the end of the committed prefix.

If the highest final marker is corrupt, its referenced generation log is missing/corrupt, or its committed-prefix proof fails, verification fails closed. It never falls back to a lower marker. This is the retained-artifact form of the no-rollback rule from `docs/append-log-generation-switch.md`.

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
- highest generation id observed in any canonical generation/final-marker/staging-marker name;
- all observed final marker generation ids;
- all observed staging-marker generation ids;
- generation-log ids that currently have no final marker;
- the marker-bound `committed_prefix` proof;
- the independent `committed_prefix_verification` report;
- the complete current append-log `log_verification` report.

The uncommitted and staging-id lists are descriptive crash/orphan evidence. The verifier does not delete either class.

## Writer-side publication now available on Unix

`db-lab-log-generation-publish` can publish a marker-v2 commit for an already-created clean generation on supported Unix targets. It synchronizes the generation file and directory before marker authority, writes and synchronizes a same-directory staging marker, publishes the final marker with no-overwrite hard-link semantics, and synchronizes the directory again before reporting success.

Canonical staging markers exist specifically so interruption before final publication has a protocol-defined non-authoritative representation. A final marker that becomes visible but whose parent-directory sync fails is reported as durability-uncertain; the tool does not claim success or delete historical state.

Non-Unix targets currently fail before writing any marker. In particular, Windows support remains intentionally disabled until an equivalent parent-directory entry durability mechanism is implemented and tested. See `docs/append-log-generation-publication.md` for the full ordering and crash-state contract.

## Compaction lifecycle status

The Unix offline compact switch now allocates above every observed canonical id, constructs the next generation from the exact authoritative live state, coordinates its authority-changing section with generation-aware routed writers, publishes the marker, and re-verifies the new authority. A deterministic composed fault matrix requires exact old-or-new logical recovery across candidate, staging-marker, and final-marker durability boundaries.

The broader lifecycle still lacks:

- Windows-equivalent durable marker publication;
- safe old-generation and orphan cleanup only after the new marker is durably committed;
- migration or coexistence rules for the current legacy single-file `LogEngine` path;
- stronger ownership if non-cooperating raw-path writers must be prevented rather than administratively quiesced.

The injected-error fixtures and modeled loss of one unsynchronized directory entry are not physical power-loss emulation. Until the remaining pieces exist and are tested against the v2 reader and executable recovery model, the Phase 1 general compaction milestone remains open.
