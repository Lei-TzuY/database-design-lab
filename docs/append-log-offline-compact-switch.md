# Offline append-log generation compact switch

`db-lab-log-generation-compact-switch` composes the shared generation-directory verifier, append-log compact-copy implementation, and durable generation-marker publisher into one **offline** authoritative switch on Unix.

```text
db-lab-log-generation-compact-switch \
  --directory data/log-generations
```

The protocol is `append_log_offline_generation_compact_switch_unix_v1`.

## Required operating condition

Every writer that can mutate any append-log generation in the directory must remain quiesced for the entire command. This is a real correctness precondition, not a performance recommendation.

The repository still exposes raw-path `LogEngine`, so the compact-switch operation cannot acquire a lock that prevents an independent process from writing directly to the old generation after the final source recheck. The operation therefore does not claim online concurrency safety. A later generation-directory engine/writer-routing layer must own that exclusion before this can become an online maintenance operation.

On non-Unix targets the operation returns unsupported before touching the supplied directory. In particular, Windows remains disabled because the repository does not yet claim an equivalent durable parent-directory publication primitive there.

## Successful order

The switch preserves the frozen generation recovery law:

1. verify `append_log_generation_directory_v2` and select only the highest final committed marker;
2. snapshot the authoritative generation's complete inspected live state and marker/recovery witness;
3. allocate the next generation id strictly above **every** observed generation log, final marker, and staging marker id;
4. build a fresh compact log at that canonical generation path through `append_log_compact_copy_v1`;
5. re-verify the generation directory and re-inspect the old authoritative source;
6. require the authoritative generation, committed-prefix witness, complete log verification, and complete live state to be unchanged;
7. independently inspect the compact target and require exact live-state equality;
8. publish its marker through `append_log_generation_marker_publication_unix_v1`;
9. re-run generation-directory verification and require the new generation to be authoritative;
10. re-inspect the new authoritative generation and require exact equality with the pre-switch live state.

The old generation and old marker are retained. This command performs no historical cleanup.

## Race and crash behavior

A new compact generation has no authority until its final marker is durably published. If the old authoritative source changes before the pre-publication recheck, the command fails with `SourceChanged`; the compact target remains only an uncommitted orphan and recovery continues to select the old committed generation.

The test suite injects a deterministic late source write after compact construction and before marker publication. It requires the switch to fail, requires the new generation file to remain uncommitted, and requires the v2 reader to keep the old generation authoritative.

The allocation frontier also treats uncommitted generation logs and staging markers as consumed ids. Crash residue is never silently reused: if the highest observed id is 7, the next switch attempts generation 8 even when only generation 1 is currently committed.

Marker publication retains its own durability-uncertain state. If the final marker becomes visible but the final parent-directory durability barrier fails, the operation returns the publication error and does not pretend the switch failed before authority could have changed. Operators must preserve retained generations and run the generation verifier before deciding how to retry.

## What this completes

This is the first concrete operation in the repository that performs the complete old-authority -> fresh compact generation -> durable marker -> new-authority sequence without subprocess composition or duplicated recovery rules.

It closes these previously separate repository-side pieces for the Unix offline case:

- authoritative source selection;
- fresh allocation above crash residue;
- exact live-state compaction;
- stale-source detection before publication;
- durable marker publication;
- post-publication authoritative/state verification.

## What remains

The broader Phase 1 compaction milestone remains open because the append-log engine still lacks lifecycle integration:

- a generation-directory `KvEngine` that routes normal mutations only to the selected authoritative generation;
- writer exclusion/locking that makes the compact switch safe as an online operation rather than requiring caller quiescence;
- Windows-equivalent durable marker publication;
- deterministic fault injection at every cross-operation durability boundary, beyond the existing component-level tests and pre-publication race injection;
- safe deletion policy for obsolete committed generations and uncommitted crash orphans;
- migration/coexistence rules for legacy one-file append-log users.

Until those exist, this command is an explicit offline maintenance primitive, not a transparent production compactor.
