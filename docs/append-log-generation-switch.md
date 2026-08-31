# Append-log generation-switch recovery law

This document freezes the recovery law for authoritative append-log compaction switching. The executable oracle defines which generation recovery may select after every interruption point. Concrete retained bytes are defined by `docs/append-log-generation-directory.md`; Unix marker publication is defined by `docs/append-log-generation-publication.md`; durable allocation reservations are defined by `docs/append-log-generation-reservations.md`; the composed offline switch is defined by `docs/append-log-offline-compact-switch.md`; generation-aware mutation routing and writer exclusion are defined by `docs/append-log-generation-routing.md`; conservative retained-history cleanup is defined by `docs/append-log-generation-cleanup.md`.

The executable oracle lives in `crates/db-storage-log/tests/generation_switch_model.rs`.

## Scope and terminology

A **generation** is a candidate append-log v1 file plus commit metadata that can make that generation authoritative. Generation ids are monotonically increasing logical identifiers. The legacy one-file append-log API remains available, while the generation-aware wrapper owns directory-level authority selection.

The model classifies a generation log as one of:

- `clean`: complete append-log v1 evidence accepted by read-only verification;
- `recoverable_tail`: the existing v1 canonical incomplete-final-append case, which mutable open may truncate and synchronize only when commit metadata proves that the complete compacted base prefix precedes that tail;
- `missing`;
- `corrupt`: anything else that the v1 verifier rejects.

A generation is **committed** only when its final commit marker is authoritative under the publication/recovery contract. A higher generation file without a final marker is an orphan candidate, not authoritative state. A durable `reserve-%020d.frontier` file records only that an id was already allocated; reservations never carry authority.

## Authoritative selection rule

Recovery MUST select the highest generation id that has a final commit marker. Directory enumeration order is irrelevant. Generation logs, staging markers, and reservation files never become authoritative without a final marker.

For that selected generation:

- `clean` -> open it directly;
- `recoverable_tail` with a marker-bound complete base prefix -> it remains authoritative and may use the existing v1 final-append repair path;
- `recoverable_tail` without proof of the committed base prefix -> fail closed;
- `missing` -> fail closed;
- `corrupt` -> fail closed.

Recovery MUST NOT fall back to a lower committed generation when the highest committed generation is missing or corrupt. Once a generation has been committed it may have accepted later synchronized mutations; fallback could silently acknowledge less state than was previously durable.

## Uncommitted generations and reservations

Higher generation ids without final commit markers never override the last committed generation, regardless of whether their files are absent, partially written/corrupt, or fully valid. They remain crash orphans until a cleanup protocol proves they are safe to remove. Recovery never infers commitment from a valid-looking generation file.

Directory v3 adds durable zero-byte reservations. A reservation contributes to the monotonic allocation frontier but does not affect recovery selection. Once reservation N is durably retained, later lifecycle work may reclaim a proven-abandoned candidate/staging artifact for N without making N reusable.

## Required writer order

Every generation switch must preserve this authority order:

1. retain the old committed generation;
2. choose a generation id that has never been reused;
3. construct the next generation without making it authoritative;
4. make the complete next-generation log durable;
5. verify the durable next-generation image;
6. capture the exact verified base-prefix byte length, CRC-32, record count, and next sequence;
7. durably publish the v2 commit marker binding that generation id and verified complete base prefix;
8. only after marker durability may old-generation cleanup become eligible.

The Unix durable reservation primitive now exists to make step 2 independent of candidate lifetime. The current offline compact switch still computes its frontier directly and does not yet create a reservation before construction; wiring that primitive into the switch is the next integration step.

The Unix offline implementation re-verifies the old authority/live state immediately before publication and re-verifies the new authority/exact compact image after publication. The authority-changing critical section is protected by the cooperative generation-writer lease also used by routed operations and the standalone publisher CLI.

Invalid writer orders include publishing the marker before the new generation is durable, publishing a marker whose prefix proof does not describe a complete verified compact image, or deleting/corrupting the old committed generation before the new marker is durable. These states fail closed rather than invoking recovery heuristics.

## Crash-state table

| Crash point | Old generation | New generation | Recovery |
| --- | --- | --- | --- |
| before new file exists | committed + clean | absent or reservation-only | old |
| during new file construction | committed + clean | uncommitted + incomplete/corrupt | old |
| after new image is complete | committed + clean | uncommitted + clean | old |
| after final marker link, before directory barrier | committed + clean | final marker may survive or be lost | old or new; verify retained state |
| after new marker is durable | committed + clean | committed + clean | new |
| marker exists but base-prefix proof fails | committed + clean | committed + unproven recoverable tail | fail closed |
| during old cleanup | missing/damaged old | committed + clean new | new |
| after old cleanup | absent old | committed + clean new | new |

A committed new generation with a canonical recoverable final append selects the new generation and delegates only that tail repair to append-log v1 when the marker proves the complete compacted base prefix is intact and the incomplete bytes follow it. Without that proof, recovery fails closed. A committed new generation that is missing or corrupt never causes re-selection of the old generation.

## Implemented repository contracts

`append_log_generation_directory_v3` provides strict canonical namespace parsing, marker-v2 decoding, generation/format binding, committed-prefix byte/CRC/record/sequence proof, source-read-only prefix verification, highest-marker/no-rollback selection, and durable reservation-frontier interpretation. V2-shaped retained directories with no reservation files remain readable; v3 adds the canonical zero-byte reservation class and `reservation_generation_ids` summary field.

`append_log_generation_reservation_unix_v1` allocates the next id under the cooperative writer lease, create-news and synchronizes a zero-byte reservation, synchronizes the generation directory, and re-verifies that the reservation is retained before reporting success. Non-Unix targets fail before reservation filesystem access.

`append_log_generation_marker_publication_unix_v1` provides Unix generation-file and parent-directory synchronization, same-directory synced staging markers, no-overwrite final-marker publication, final directory synchronization, post-publication proof verification, and a distinct durability-uncertain error when final-marker visibility may precede confirmed parent-directory durability. Non-Unix platforms fail before publication I/O.

`append_log_offline_generation_compact_switch_unix_v1` composes source selection, allocation above every observed canonical id, exact live-state compact-copy construction, stale-source detection, durable publication, and final authority/image verification. A deterministic late-write test proves pre-publication raw-path source drift leaves only an uncommitted orphan and preserves old authority. A composed fault matrix covers the complete candidate, locked recheck, generation durability, partial and durable staging marker, final-marker link, final directory barrier, and completed-publisher boundaries. Normal recovery must select an exact logical old or new state according to the table above. Its next integration is to create a durable reservation before candidate construction rather than relying on candidate/staging names alone for non-reuse evidence.

`GenerationLogEngine` adds generation-aware normal mutation routing. A handle adopts a newly published higher final marker before its next operation and refuses marker regression or malformed higher authority rather than continuing on a stale old generation.

`append_log_generation_writer_lock_v1` adds cooperative cross-process exclusion. Each routed operation holds a create-new sibling lock from before authority refresh through the operation. Compact-switch publication acquires the same lease before its final source/authority recheck and keeps it through marker publication and final verification. The standalone marker-publisher CLI also acquires the lease. A stale crash lock is never stolen automatically; it fails closed until explicitly cleared after operator verification.

`append_log_generation_cleanup_unix_v1` adds conservative retained-history cleanup under that same writer lease. It deletes generation logs and final markers only below current authority, plus staging markers only at or below current authority. Higher staging-marker ids and higher uncommitted generation files are still retained because current compact candidate construction can occur outside the publication lease and has not yet been migrated to durable pre-construction reservations. Lower markers are removed and the generation directory synchronized before lower generation logs are removed and synchronized. The current authority is re-verified before deletion, between metadata/data phases, and after cleanup.

The writer lock deliberately lives outside the retained generation namespace, so transient coordination does not alter recovery evidence or generation-id allocation. Raw-path `LogEngine` users do not participate and remain outside the exclusion contract.

Hosted-CI fixtures validate format/recovery, reservation semantics, composed interruption states, cooperative exclusion semantics, and cleanup state transitions. The pre-directory-sync final-link case explicitly permits old or new: the tests exercise a visible marker and a modeled loss of that unsynchronized directory entry, not a physical power-loss emulator. Hosted CI does not by itself prove real power-loss durability of arbitrary filesystems or prevent a process that intentionally bypasses the protocol.

## What remains before broad Phase 1 compaction completion

The remaining lifecycle work is narrower but still material:

- wire durable reservations into compact-switch candidate allocation before construction;
- Windows-equivalent durable final-marker/reservation publication;
- guarded reclamation of confirmed-abandoned higher generation/staging artifacts while preserving their reservation frontier evidence;
- migration/coexistence rules for legacy one-file users and a decision about where the generation-directory contract ultimately lives in the crate layering;
- stronger ownership if the project later wants to protect against non-cooperating raw-path writers rather than only generation-aware participants.

The broad roadmap `Compaction` item remains intentionally open until the remaining lifecycle pieces are implemented, even though Unix now has an authoritative compact switch, generation-aware mutation routing, cooperative cross-process writer exclusion, conservative obsolete-history cleanup, and a durable generation-id reservation primitive.
