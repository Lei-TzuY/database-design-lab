# Append-log generation-aware mutation routing

`db_cli::generation_engine::GenerationLogEngine` owns a **generation directory** instead of requiring callers to keep and mutate a raw `generation-%020d.log` path.

It implements the common `KvEngine` contract and reports the same append-log logical/storage semantics under the stable name `append-log-generation-v2`.

## Open and recovery

`GenerationLogEngine::open(directory)` acquires the cooperative generation-writer lease before running the shared `append_log_generation_directory_v2` verifier and mutable open. Only the highest final committed marker may select authority; missing/corrupt highest committed generations fail closed and the wrapper never falls back to an older committed generation.

Only after the retained marker and committed-prefix proof succeed does the wrapper open the selected append-log file mutably. This preserves the existing rule for a canonical incomplete final append: `LogEngine::open` may repair it only after the generation verifier has proven that the marker-bound compact base prefix is intact and the tail begins after that prefix. The directory is verified again after mutable open/repair and authority must still name the same generation.

## Per-operation routing and exclusion

Before every `GET`, `PUT`, `DELETE`, or `REOPEN`, the wrapper acquires the same generation-writer lease used by compact-switch publication. The lease is held through authority refresh and the entire operation, then released. Long-lived routing handles therefore do **not** monopolize the lease between calls.

After acquiring the lease, ordinary operations inspect the highest **final marker id**:

- same id as the current inner handle: continue on that handle;
- higher id: run full generation-directory verification and open the newly selected generation before executing the operation;
- lower id or no final marker: fail closed as forbidden rollback;
- malformed/corrupt higher marker or generation: full verification fails and the routing handle is poisoned.

A poisoned routing handle rejects ordinary operations with `DbError::Poisoned`. Explicit `reopen()` is the recovery boundary: it re-runs the full generation-directory verifier, refuses any authority lower than the generation already observed by the handle, and clears poison only after a valid same-or-higher authority is opened.

The lease is a create-new sibling file of the canonical generation directory, for example `data/generations` uses `data/.generations.append-log-writer.lock`. It deliberately lives **outside** the retained generation namespace, so retained evidence contains only generation logs and marker artifacts. The lock record contains a protocol tag and process id for operator diagnosis, but the implementation never trusts PID or age to steal a lock.

A crashed process may leave a stale lock. That is a liveness failure, not a reason to risk two writers: subsequent coordinated operations fail closed until an operator independently proves no writer is alive and removes the stale sibling lock. Normal drop performs best-effort lock removal.

## Compact-switch interaction

`compact_switch_generation_offline` builds the expensive compact candidate without holding the writer lease. This keeps normal routed operations available during the copy phase. Immediately before authority can change, the switch acquires the lease and, while holding it:

1. re-verifies the old authoritative generation and its full live state;
2. rejects the switch if any compliant writer changed the source during copy construction;
3. re-verifies the compact candidate;
4. durably publishes the new final marker;
5. verifies the new authority and exact compact image.

If a routed writer changed the source before the switch acquired the lease, the locked recheck detects the drift and the candidate remains an uncommitted orphan. If a routed writer tries to operate while the switch holds the lease, it receives a `WouldBlock`-class error before authority refresh or append. After the switch releases the lease, an existing routed handle lazily adopts the new generation on its next operation.

This closes the previous cooperative scan-to-append / final-check-to-publication race between `GenerationLogEngine`, the standalone publisher CLI, and the offline compact switch.

## Boundary

This is **cooperative** cross-process exclusion, not protection against arbitrary raw-file mutation. A process that directly opens `generation-%020d.log` with `LogEngine` bypasses the generation-directory lease and remains outside the contract. Such raw-path writers must still be quiesced during a compact switch.

The standalone `db-lab-log-generation-publish` CLI now acquires the same lease before invoking the lower-level marker publication primitive. The lower-level shared publisher remains a caller-serialized primitive so the compact switch can call it while already holding the lease without self-deadlock.

The project still does not claim Windows final-marker durability, automatic stale-lock recovery, old-generation/orphan cleanup, or adversarial protection from a process that intentionally ignores the coordination protocol.

## Tests

Cross-platform integration proves routed CRUD/reopen behavior, no rollback after higher authority, malformed-higher-marker poisoning, and recovery after marker restoration. Writer-exclusion coverage additionally proves:

- an externally held lease blocks a routed mutation before any bytes are appended;
- stale lock evidence is not rewritten or automatically stolen;
- the lease file does not enter the retained generation namespace;
- on Unix, a compact switch cannot publish while another lease is held;
- the failed switch leaves only an uncommitted generation orphan, and retry allocates above that orphan rather than reusing its id.

Unix integration continues to prove that a long-lived routed handle can survive a completed offline compact switch and send its first later mutation only to the newly authoritative generation.
