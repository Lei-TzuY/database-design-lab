# Append-log generation-aware mutation routing

`db_cli::generation_engine::GenerationLogEngine` is the first append-log engine boundary that owns a **generation directory** instead of requiring callers to keep and mutate a raw `generation-%020d.log` path.

It implements the common `KvEngine` contract and reports the same append-log logical/storage semantics under the stable name `append-log-generation-v2`.

## Open and recovery

`GenerationLogEngine::open(directory)` first runs the shared `append_log_generation_directory_v2` verifier. Only the highest final committed marker may select authority; missing/corrupt highest committed generations fail closed and the wrapper never falls back to an older committed generation.

Only after that retained marker and committed-prefix proof succeed does the wrapper open the selected append-log file mutably. This preserves the existing rule for a canonical incomplete final append: `LogEngine::open` may repair it only after the generation verifier has proven that the marker-bound compact base prefix is intact and the tail begins after that prefix. The directory is verified again after mutable open/repair and authority must still name the same generation.

## Per-operation routing

Before each `GET`, `PUT`, or `DELETE`, the wrapper scans the strict canonical namespace and reads the highest **final marker id**:

- same id as the current inner handle: continue on that handle;
- higher id: run full generation-directory verification and open the newly selected generation before executing the operation;
- lower id or no final marker: fail closed as forbidden rollback;
- malformed/corrupt higher marker or generation: full verification fails and the routing handle is poisoned.

A poisoned routing handle rejects ordinary operations with `DbError::Poisoned`. Explicit `reopen()` is the recovery boundary: it re-runs the full generation-directory verifier, refuses any authority lower than the generation already observed by the handle, and clears poison only after a valid same-or-higher authority is opened.

This closes the stale-handle lifecycle gap after the offline compact switch. A handle that was routed to generation 1 can remain allocated while an operator, under caller quiescence, performs `compact_switch_generation_offline`. Its next operation observes final marker 2, opens generation 2, and never appends that operation to the stale generation-1 file.

## What the fast path does and does not verify

When the highest final marker id is unchanged, the per-operation fast path does not replay the whole current log or recompute its committed-prefix CRC. That would turn every point mutation/read into an O(database-size) verification pass. Full retained-artifact verification occurs on initial open, explicit reopen, and every observed authority advance.

The engine keeps the repository's existing `CallerSerialized` concurrency contract. External mutation of current marker bytes or same-generation log bytes while a handle is active is outside that contract and is not continuously polled for integrity.

## Concurrency boundary

Namespace routing is **not a cross-process lock**. There remains a scan-to-append window in which another process that ignores caller serialization could publish a higher marker after this handle checks the marker frontier but before it appends to the old generation.

For the same reason, `compact_switch_generation_offline` still requires all writers to be quiesced for its full call. This routing layer makes the post-switch owner path correct, but it does not yet make compaction an online concurrent maintenance operation.

The next lifecycle layer must provide one ownership/exclusion mechanism around normal mutations and compact-switch publication before the broad Phase 1 compaction milestone can be considered complete.

## Tests

Cross-platform integration coverage constructs valid retained generation/marker evidence without claiming marker-publication durability, then proves routed `GET`/`PUT`/`DELETE`/`REOPEN` behavior. Additional adversarial cases prove:

- removing the marker of an already observed higher generation never rolls the handle back to an older generation;
- a malformed higher marker poisons the handle before any stale-generation mutation occurs;
- after restoring valid authority, explicit reopen can recover a poisoned handle without rollback.

Unix integration additionally composes the actual durable publisher and offline compact switch, keeps the original routing handle alive but quiescent during the switch, then proves its first post-switch mutation lands only on the newly authoritative generation.
