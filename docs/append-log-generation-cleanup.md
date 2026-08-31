# Append-log generation cleanup

`db-lab-log-generation-cleanup` is the conservative retained-history cleanup primitive for the generation-directory append log.

```text
db-lab-log-generation-cleanup --directory data/log-generations
```

The current protocol is `append_log_generation_cleanup_unix_v1`. It is Unix-only because a successful result includes parent-directory durability barriers for removed names. Unsupported platforms fail before filesystem access.

## Safety rule

Cleanup never chooses authority. It first acquires the same cooperative writer lease used by generation-aware mutations and compact-switch publication, then runs the normal generation-directory verifier. The highest committed generation selected by that verifier must remain byte-for-byte equivalent in its authority witness throughout cleanup.

The automatic deletion set is intentionally narrow:

- every final `commit-%020d.marker` with generation id lower than the current authoritative generation;
- every `generation-%020d.log` with generation id lower than the current authoritative generation, including a lower log whose old marker was already removed by an earlier interrupted cleanup;
- canonical `staging-commit-%020d.marker` names only when their generation id is at or below current authority.

Higher staging-marker ids remain outside **automatic** cleanup even when a durable reservation covers their id. Reservation evidence proves identity retirement, not that a live compactor has abandoned the artifact.

Cleanup also deliberately retains every uncommitted generation log at or above the current authoritative generation. Compact candidate construction occurs outside the compact-switch publication lease, so a higher uncommitted generation may still be under construction. The automatic cleanup protocol does not guess whether such a candidate is abandoned.

When an operator has independently established abandonment, `db-lab-log-generation-abandon-cleanup` provides a separate plan/review/explicit-confirmation protocol. It requires a matching durable reservation, binds the exact directory snapshot and candidate/staging bytes, replays that plan under the writer lease, and leaves the reservation intact so deletion cannot cause id reuse. See `docs/append-log-abandoned-generation-cleanup.md`.

A lower generation can never become authoritative again: final-marker publication is monotonic and requires a generation newer than every existing committed marker. Removing lower retained history therefore cannot create a future valid publication path back to that generation, and current authority itself preserves the allocation frontier above all removed lower ids.

## Validation before deletion

The complete deletion plan is computed before the first removal. Every planned marker, staging marker, and generation log must still be a real regular file. Symlinks and non-files fail closed before cleanup starts.

The current authority is then re-verified immediately before deletion. Generation-aware writers and publisher critical sections are excluded by the cooperative lease. Direct raw-path `LogEngine` users remain outside that contract and must not mutate the generation directory during maintenance.

## Durable deletion order

On Unix the successful sequence is:

1. acquire the generation writer lease;
2. verify and capture the current authoritative generation witness;
3. scan and validate the complete cleanup plan;
4. re-verify the same authority;
5. remove all lower final markers and staging-marker names whose id is at or below current authority;
6. `sync_all` the generation directory;
7. re-verify the same current authority before deleting data files;
8. remove all generation logs whose id is lower than current authority;
9. `sync_all` the generation directory again;
10. re-verify the same authority and report retained higher staging/uncommitted generation ids.

The marker-first order is load-bearing for interrupted cleanup. Once a lower marker deletion is durably recorded, its lower generation log is only orphan history. If the process crashes before the data-file deletion barrier, recovery still selects the unchanged highest committed generation.

If a directory durability barrier fails after visible removals, cleanup returns a distinct durability-uncertain error rather than claiming successful reclamation. The visible state is still recovery-safe: interrupted deletion can only retain or remove history below the unchanged authority, or remove staging evidence whose id is already covered by current authority.

## Idempotence and partial prior cleanup

A later cleanup can finish safe lower-generation residue from an earlier interruption. Examples include a lower generation log whose final marker is already absent, or a lower final marker whose corresponding generation log is already absent. Neither lower artifact participates in highest-marker recovery, so the remaining obsolete name can be removed on the next successful run.

Higher staging markers and higher uncommitted generation logs are reported but retained by this automatic command. Reserved higher artifacts can be handled only through the separate explicit abandoned-artifact cleanup protocol; unreserved higher artifacts remain ineligible because deleting them could lower the known allocation frontier.

## What this does not do

This automatic cleanup does not:

- delete the current authoritative generation or marker;
- infer that a higher orphan is abandoned from PID, age, timestamp, or filename alone;
- delete unreserved higher candidate/staging frontier evidence;
- provide Windows-equivalent directory-entry durability;
- migrate legacy single-file append-log users;
- protect against non-cooperating direct raw-path mutation.

The broad Phase 1 compaction milestone therefore remains open after this cleanup slice. The separate explicit abandoned-artifact protocol closes only the reserved higher-artifact lifecycle case; Windows durability, automatic abandonment proof, legacy single-file migration/coexistence, and non-cooperating raw writers remain outside the current contract.
