# Append-log generation-switch recovery law

This document freezes the recovery law that a future authoritative append-log compaction switch must implement. It is intentionally one step earlier than a filesystem protocol: it defines which generation recovery is allowed to select after every interruption point, without yet assigning marker filenames, marker bytes, or directory-fsync mechanics.

The executable oracle lives in `crates/db-storage-log/tests/generation_switch_model.rs`.

## Scope and terminology

A **generation** is a candidate append-log v1 file plus durable commit metadata that can make that generation authoritative. Generation ids are monotonically increasing logical identifiers in the future protocol. The current one-file append-log remains unchanged by this PR.

The model classifies a generation log as one of:

- `clean`: complete append-log v1 evidence accepted by read-only verification;
- `recoverable_tail`: the existing v1 canonical incomplete-final-append case, which mutable open may truncate and synchronize only when commit metadata proves that the complete compacted base prefix precedes that tail;
- `missing`;
- `corrupt`: anything else that the v1 verifier rejects.

A generation is **committed** only after the future protocol's commit marker is durably published. A higher generation file with no durable marker is an orphan candidate, not authoritative state.

## Authoritative selection rule

Recovery MUST select the highest generation id that has a durable commit marker. Directory enumeration order is irrelevant.

For that selected generation:

- `clean` -> open it directly;
- `recoverable_tail` with a marker-bound complete base prefix -> it remains authoritative and may use the existing v1 final-append repair path;
- `recoverable_tail` without proof of the committed base prefix -> fail closed;
- `missing` -> fail closed;
- `corrupt` -> fail closed.

Recovery MUST NOT fall back to a lower committed generation when the highest committed generation is missing or corrupt.

That no-fallback rule is deliberate. Once a generation has been committed it may have accepted later synchronized mutations. Falling back to an older generation after discovering damage in the highest committed generation could silently acknowledge less state than was previously durable.

## Uncommitted generations

Higher generation ids without durable commit markers never override the last committed generation, regardless of whether their files are:

- absent;
- partially written or corrupt;
- fully constructed and valid.

Those files are crash orphans until a future cleanup protocol proves they are safe to remove. Recovery selection does not infer commitment from a valid-looking generation file.

## Required writer order

Any future generation switch must preserve this order:

1. retain the old committed generation;
2. construct the next generation without making it authoritative;
3. make the complete next-generation log durable;
4. verify the durable next-generation image;
5. durably publish a commit marker that binds the generation id and verified complete base prefix;
6. only after marker durability may old-generation cleanup become eligible.

The executable model covers every valid prefix of that sequence and requires recovery to choose exactly the old or new generation.

Two writer orders are explicitly invalid:

- publishing the new marker before the new generation is durable;
- publishing a marker that does not prove the complete compacted base prefix;
- deleting or corrupting the old committed generation before the new marker is durable.

The model treats the resulting states as fail-closed conditions rather than inventing recovery heuristics.

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

A committed new generation with a canonical recoverable final append selects the new generation and delegates only that tail repair to the existing append-log v1 recovery rule **only** when the marker proves that the verified complete compacted base prefix is intact and the incomplete bytes follow it. Without that proof, recovery fails closed: the same apparent tail could be an incomplete compact image published too early.

A committed new generation that is missing or corrupt fails closed. It never reselects the old generation.

## What this PR does not implement

This recovery law is an executable design oracle, not production generation switching. It does not yet define or implement:

- generation/marker filenames;
- the marker byte format, versioning, checksums, bounds, or exact committed-prefix binding;
- the exact file and directory `sync_all` sequence;
- cross-platform create-new and rename/link mechanics for marker publication;
- orphan discovery and cleanup;
- migration from the current one-file layout;
- mutation routing after the authoritative generation changes;
- an engine API that opens a generation directory instead of one v1 file.

Those mechanics must be implemented in a later PR and adversarially tested against this frozen recovery law. The general Phase 1 compaction milestone remains incomplete until an authoritative switch exists on the default branch.
