# Append-log generation cleanup

`db-lab-log-generation-cleanup` implements `append_log_generation_cleanup_v1`, a conservative reclamation pass for `append_log_generation_directory_v2`.

```text
db-lab-log-generation-cleanup --directory data/log-generations
```

The cleanup contract is intentionally asymmetric around current authority. If generation `N` is authoritative, only canonical generation logs, final markers, and staging markers with ids **strictly lower than `N`** are eligible for deletion. Generation `N` itself and every canonical id greater than `N` are retained.

## Why higher uncommitted ids are retained

The generation allocator chooses one greater than the highest observed canonical generation/log/marker/staging id. A failed compact switch can therefore leave a valid-looking but uncommitted generation above current authority. Deleting that higher orphan immediately would lower the observed allocation frontier and permit the same generation id to be reused.

This cleanup avoids introducing a new persistent high-watermark format. Higher crash residue remains until a later successful switch allocates above it and makes an even higher generation authoritative. At that point the former orphan is below authority and becomes safely reclaimable on the next cleanup pass.

Example:

1. generation 1 is authoritative;
2. failed switch leaves uncommitted generation 2;
3. cleanup preserves generation 2;
4. retry allocates generation 3 and commits it;
5. cleanup may now delete generation 1, marker 1, and orphan generation 2 because all are below authority 3.

## Exclusion and preflight

Cleanup acquires the same cooperative writer lease used by `GenerationLogEngine` operations and compact-switch publication. If the lease is held or stale, cleanup fails before deletion.

While holding the lease, cleanup:

1. fully verifies current generation-directory authority;
2. scans the strict canonical namespace;
3. identifies only ids lower than authority;
4. pre-validates **every** eligible deletion target as a real regular file before deleting anything;
5. removes obsolete lower final markers, lower staging markers, then lower generation logs;
6. on Unix, synchronizes the parent generation directory after visible deletions;
7. re-verifies that the same authoritative generation, committed prefix, and current log verification remain intact;
8. reports retained higher generation/staging ids and the before/after allocation frontier.

The marker-first deletion order makes interruption conservative: an old lower log may remain without its lower marker, but cleanup never intentionally leaves a lower marker naming a log it already removed. Either partial state remains irrelevant because only the highest final committed marker selects authority.

## Crash and durability behavior

No cleanup deletion is required for correctness. If a crash causes some deleted lower names to reappear, they remain below current authority and cannot override it. Partial cleanup is therefore a space-reclamation/liveness concern rather than a recovery-selection ambiguity.

Unix calls `sync_all` on the generation directory after the deletion batch and reports `directory_sync_confirmed=true` only when that barrier succeeds. Other platforms do not claim an equivalent directory-entry durability guarantee for cleanup and report `false`; stale lower files after a crash remain semantically harmless.

If the Unix directory sync fails after visible deletions, the operation returns a durability-uncertain error rather than claiming persistent reclamation. The authoritative generation is never selected by cleanup and is never an eligible deletion target.

## Safety boundary

The writer lease is cooperative. A process that directly mutates or removes retained files through raw filesystem or raw-path `LogEngine` access can bypass it. Cleanup protects against generation-aware participants, not a hostile process ignoring the protocol.

Cleanup also deliberately does not remove higher uncommitted generations or staging markers, automatically steal stale writer locks, migrate legacy single-file logs, or provide Windows-equivalent final-marker publication durability.
