# Append-log durable generation reservations

`append_log_generation_directory_v3` adds one retained, non-authoritative namespace class:

```text
reserve-00000000000000000042.frontier
```

A reservation is a zero-byte real regular file whose generation id contributes only to the monotonic allocation frontier. It is not a commit marker, is never selected as authority, and does not change append-log file format v1 or commit-marker format v2.

The reservation command is:

```text
db-lab-log-generation-reserve --directory data/log-generations
```

On supported Unix targets it reports protocol `append_log_generation_reservation_unix_v1`.

## Why reservations exist

Before this protocol the next compact generation id was derived from the highest generation id visible in generation logs, final markers, or staging markers. That prevented accidental id reuse while crash residue remained in the directory, but it also made higher abandoned residue undeletable: deleting the last artifact carrying a high id could lower the observed frontier and permit later reuse.

A durable reservation separates these concerns. The id is reserved before a candidate generation is built. Candidate, staging, and other non-authoritative residue may later become eligible for guarded cleanup while the reservation continues to prove that the id was already allocated.

Reservations also give concurrent cooperative compactors a proper allocation primitive. Allocation occurs while holding the same generation writer lease used by routed mutations and authority-changing publication. Two cooperating allocators therefore cannot both successfully reserve the same generation id.

## Unix durability order

`reserve_next_generation` performs:

1. acquire the cooperative generation writer lease;
2. verify the current generation directory through the shared v3 reader;
3. choose `highest_observed_generation + 1` with checked `u64` arithmetic;
4. create-new the canonical zero-byte reservation file;
5. `sync_all` the reservation file;
6. `sync_all` the generation directory;
7. re-run the shared verifier and require the new reservation id to be retained in the allocation frontier;
8. release the writer lease.

The operation reports success only after the directory durability barrier and retained-state re-verification complete.

If the reservation becomes visible but the parent-directory sync fails, the operation returns a durability-uncertain error. Callers must not construct a candidate from that failed reservation result. If the visible reservation survives, later allocation conservatively skips it; if it does not survive a crash, no successful caller had been permitted to rely on it.

## Read-side validation

The v3 generation-directory reader accepts reservation names in addition to the v2 generation/final-marker/staging-marker namespace. A reservation must:

- use exactly `reserve-%020d.frontier` with a nonzero canonical decimal `u64` id;
- be a real regular file, not a symlink or other file type;
- contain exactly zero bytes.

Malformed, non-file, or nonempty reservation evidence fails directory verification closed.

`highest_observed_generation` now considers generation logs, final markers, staging markers, and reservation ids. Successful JSON also includes `reservation_generation_ids`.

Reservation contents carry no authority and are intentionally empty. The filename and its durably retained directory entry are the monotonic allocation evidence.

## Compatibility and versioning

Directory protocol v2 is the frozen predecessor that knew only generation logs, final markers, and staging markers. The v3 reader remains able to read a retained v2-shaped directory containing no reservations; it reports an empty reservation list. The protocol identifier is nevertheless v3 because accepting a new canonical namespace class and changing allocation-frontier semantics is a real retained-format contract change.

Commit-marker format remains version 2 and append-log file format remains version 1.

## Platform boundary

Durable reservation publication is currently Unix-only because success depends on a parent-directory durability barrier. Non-Unix targets fail before filesystem access rather than claiming an unproven equivalent.

Read-only v3 directory verification remains cross-platform.

## Compact-switch integration

`append_log_offline_generation_compact_switch_unix_v2` now calls the reservation primitive before candidate construction. The switch therefore has two deliberately separate lease windows:

1. reserve a fresh generation id durably under the shared writer lease, then release it;
2. build the expensive compact candidate without monopolizing writers;
3. reacquire the same lease for the final old-authority/live-state recheck through durable marker publication and final verification.

A successful switch reports its `GenerationReservationSummary` in the JSON result. If candidate construction, source-drift detection, or marker publication later fails, the reservation remains retained and that generation id stays consumed. Retry allocates above it rather than reusing an identity whose candidate may have existed before the failure.

If authority advances after reservation but before the switch snapshots its source and reaches or exceeds the reserved id, the switch fails early with `ReservedGenerationObsolete`; it does not build or publish a candidate under an id that is no longer newer than authority. A higher reservation alone does not invalidate an older still-uncommitted reservation, because reservations carry no authority.

## Lifecycle boundary

Reservation-backed compact switching now makes abandoned candidate/staging identity independently reclaimable from allocation history: the reservation can preserve non-reuse even if those artifacts are later deleted.

The remaining cleanup problem is therefore narrower. A future guarded orphan-cleanup protocol must still establish that a higher uncommitted candidate or staging artifact is actually abandoned before deleting it; reservation existence proves identity retirement, not process liveness or abandonment. Windows-equivalent durable reservation/marker publication and legacy single-file migration/coexistence also remain unresolved.
