# Append-log generation-switch recovery law

This document freezes the recovery law for authoritative append-log compaction switching. The executable oracle defines which generation recovery may select after every interruption point. Concrete retained bytes are defined by `docs/append-log-generation-directory.md`; Unix marker publication is defined by `docs/append-log-generation-publication.md`; the composed offline switch is defined by `docs/append-log-offline-compact-switch.md`; generation-aware mutation routing and writer exclusion are defined by `docs/append-log-generation-routing.md`.

The executable oracle lives in `crates/db-storage-log/tests/generation_switch_model.rs`.

## Scope and terminology

A **generation** is a candidate append-log v1 file plus commit metadata that can make that generation authoritative. Generation ids are monotonically increasing logical identifiers. The legacy one-file append-log API remains available, while the generation-aware wrapper owns directory-level authority selection.

The model classifies a generation log as one of:

- `clean`: complete append-log v1 evidence accepted by read-only verification;
- `recoverable_tail`: the existing v1 canonical incomplete-final-append case, which mutable open may truncate and synchronize only when commit metadata proves that the complete compacted base prefix precedes that tail;
- `missing`;
- `corrupt`: anything else that the v1 verifier rejects.

A generation is **committed** only when its final commit marker is authoritative under the publication/recovery contract. A higher generation file without a final marker is an orphan candidate, not authoritative state.

## Authoritative selection rule

Recovery MUST select the highest generation id that has a final commit marker. Directory enumeration order is irrelevant.

For that selected generation:

- `clean` -> open it directly;
- `recoverable_tail` with a marker-bound complete base prefix -> it remains authoritative and may use the existing v1 final-append repair path;
- `recoverable_tail` without proof of the committed base prefix -> fail closed;
- `missing` -> fail closed;
- `corrupt` -> fail closed.

Recovery MUST NOT fall back to a lower committed generation when the highest committed generation is missing or corrupt. Once a generation has been committed it may have accepted later synchronized mutations; fallback could silently acknowledge less state than was previously durable.

## Uncommitted generations

Higher generation ids without final commit markers never override the last committed generation, regardless of whether their files are absent, partially written/corrupt, or fully valid. They remain crash orphans until a cleanup protocol proves they are safe to remove. Recovery never infers commitment from a valid-looking generation file.

## Required writer order

Every generation switch must preserve this order:

1. retain the old committed generation;
2. construct the next generation without making it authoritative;
3. make the complete next-generation log durable;
4. verify the durable next-generation image;
5. capture the exact verified base-prefix byte length, CRC-32, record count, and next sequence;
6. durably publish the v2 commit marker binding that generation id and verified complete base prefix;
7. only after marker durability may old-generation cleanup become eligible.

The Unix offline implementation re-verifies the old authority/live state immediately before publication and re-verifies the new authority/exact compact image after publication. The authority-changing critical section is now protected by the cooperative generation-writer lease also used by routed operations and the standalone publisher CLI.

Invalid writer orders include publishing the marker before the new generation is durable, publishing a marker whose prefix proof does not describe a complete verified compact image, or deleting/corrupting the old committed generation before the new marker is durable. These states fail closed rather than invoking recovery heuristics.

## Crash-state table

| Crash point | Old generation | New generation | Recovery |
| --- | --- | --- | --- |
| before new file exists | committed + clean | absent | old |
| during new file construction | committed + clean | uncommitted + incomplete/corrupt | old |
| after new image is complete | committed + clean | uncommitted + clean | old |
| after new marker is durable | committed + clean | committed + clean | new |
| marker exists but base-prefix proof fails | committed + clean | committed + unproven recoverable tail | fail closed |
| during old cleanup | missing/damaged old | committed + clean new | new |
| after old cleanup | absent old | committed + clean new | new |

A committed new generation with a canonical recoverable final append selects the new generation and delegates only that tail repair to append-log v1 when the marker proves the complete compacted base prefix is intact and the incomplete bytes follow it. Without that proof, recovery fails closed. A committed new generation that is missing or corrupt never causes re-selection of the old generation.

## Implemented repository contracts

`append_log_generation_directory_v2` provides strict canonical namespace parsing, marker-v2 decoding, generation/format binding, committed-prefix byte/CRC/record/sequence proof, source-read-only prefix verification, and highest-marker/no-rollback selection.

`append_log_generation_marker_publication_unix_v1` provides Unix generation-file and parent-directory synchronization, same-directory synced staging markers, no-overwrite final-marker publication, final directory synchronization, post-publication proof verification, and a distinct durability-uncertain error when final-marker visibility may precede confirmed parent-directory durability. Non-Unix platforms fail before publication I/O.

`append_log_offline_generation_compact_switch_unix_v1` composes source selection, allocation above every observed canonical id, exact live-state compact-copy construction, stale-source detection, durable publication, and final authority/image verification. A deterministic late-write test proves pre-publication raw-path source drift leaves only an uncommitted orphan and preserves old authority.

`GenerationLogEngine` adds generation-aware normal mutation routing. A handle adopts a newly published higher final marker before its next operation and refuses marker regression or malformed higher authority rather than continuing on a stale old generation.

`append_log_generation_writer_lock_v1` adds cooperative cross-process exclusion. Each routed operation holds a create-new sibling lock from before authority refresh through the operation. Compact-switch publication acquires the same lease before its final source/authority recheck and keeps it through marker publication and final verification. The standalone marker-publisher CLI also acquires the lease. A stale crash lock is never stolen automatically; it fails closed until explicitly cleared after operator verification.

The lock deliberately lives outside the retained generation namespace, so transient coordination does not alter recovery evidence or generation-id allocation. Raw-path `LogEngine` users do not participate and remain outside the exclusion contract.

Hosted-CI fixtures validate format/recovery and cooperative exclusion semantics. They do not by themselves prove real power-loss durability of arbitrary filesystems or prevent a process that intentionally bypasses the protocol.

## What remains before broad Phase 1 compaction completion

The remaining lifecycle work is narrower but still material:

- Windows-equivalent durable final-marker publication;
- cross-operation deterministic fault injection at the composed switch boundaries beyond existing component-level durability tests;
- safe cleanup of obsolete committed generations and uncommitted crash orphans;
- explicit stale-lock recovery/operator tooling and lifecycle evidence;
- migration/coexistence rules for legacy one-file users and a decision about where the generation-directory contract ultimately lives in the crate layering;
- stronger ownership if the project later wants to protect against non-cooperating raw-path writers rather than only generation-aware participants.

The broad roadmap `Compaction` item remains intentionally open until the remaining lifecycle pieces are implemented, even though Unix now has an authoritative compact switch, generation-aware mutation routing, and cooperative cross-process writer exclusion.
