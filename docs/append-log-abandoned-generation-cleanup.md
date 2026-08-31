# Explicit abandoned-generation cleanup

`db-lab-log-generation-abandon-cleanup` is the destructive companion to the conservative automatic generation cleanup. It exists for one narrow case: a higher uncommitted generation candidate or staging marker is known by the operator to be abandoned, while a durable reservation already proves that its generation id must never be reused.

The command is deliberately two-step:

```text
db-lab-log-generation-abandon-cleanup plan \
  --directory data/log-generations \
  > abandoned-plan.json

# Review the exact plan and establish operationally that every eligible artifact is abandoned.
db-lab-log-generation-abandon-cleanup apply \
  --directory data/log-generations \
  --plan abandoned-plan.json \
  --confirm-abandoned
```

Plan protocol is `append_log_abandoned_generation_cleanup_plan_v1`. Destructive Unix application reports `append_log_abandoned_generation_cleanup_unix_v1`.

## Eligibility is intentionally narrow

An artifact generation id is eligible only when all of the following hold in one verified directory snapshot:

- the id is strictly greater than current authoritative generation;
- no final commit marker exists for that id;
- a durable canonical `reserve-%020d.frontier` exists for exactly that id;
- at least one canonical candidate `generation-%020d.log` or `staging-commit-%020d.marker` exists.

The reservation is load-bearing: deletion may remove candidate/staging names without lowering the durable allocation frontier. The reservation itself is never removed.

A higher candidate or staging marker without a matching reservation is reported in `blocked_unreserved_*` and is never eligible. This command does not infer safety from age, PID, mtime, a valid-looking append log, or a high generation number.

## What the plan binds

The JSON plan is the confirmation object; there is no short checksum token whose collision could identify the wrong destructive plan.

The plan records:

- the generation-directory protocol and marker format;
- authoritative generation and canonical authoritative log;
- highest observed generation;
- complete final-marker, staging-marker, reservation, and uncommitted-generation id lists;
- marker-bound committed-prefix evidence;
- independent committed-prefix verification summary;
- current authoritative-log verification summary;
- every eligible generation id and its reservation name;
- candidate and staging artifact filename, exact byte length, and streaming CRC-32/IEEE;
- unreserved higher candidate/staging ids that are blocked from cleanup.

`apply` reacquires the shared writer lease and recomputes this complete plan. Any difference produces `PlanChanged` before the first deletion. This includes authority advancement, an authoritative append, a new reservation, candidate/staging byte drift, publication of a final marker, or any other namespace change represented by the plan.

The saved plan file is strict JSON (`deny_unknown_fields`), must be a real regular file, and is bounded before decoding.

## Explicit abandonment is an operator statement

A durable reservation proves only that an id is retired. It does **not** prove that no live compactor still owns a candidate with that id.

`--confirm-abandoned` therefore means the operator has established outside this protocol that every eligible candidate/staging artifact in the exact plan is no longer owned by a live operation. The tool does not guess process liveness from PID reuse, timestamps, or filesystem age.

The cooperative writer lease prevents routed mutations, marker publication, reservation, automatic cleanup, and other lease-aware critical sections from crossing destructive apply. Candidate construction intentionally occurs outside the compact-switch lease, so a still-running builder is an operator-error case. Even then, deleting an uncommitted candidate cannot make it authoritative; the builder must fail when its expected path/evidence is gone. Non-cooperating raw-path filesystem mutation remains outside the protocol contract.

## Durable deletion order

On Unix, after exact plan replay succeeds:

1. retain the cooperative writer lease;
2. immediately re-fingerprint each planned staging marker before unlinking it;
3. delete planned staging-marker names;
4. `sync_all` the generation directory;
5. re-verify that the same authority witness remains selected;
6. immediately re-fingerprint each planned generation candidate before unlinking it;
7. delete planned candidate names;
8. `sync_all` the generation directory again;
9. re-verify the same authority witness;
10. require every cleaned generation id's durable reservation still to be retained.

A failed parent-directory durability barrier returns a distinct durability-uncertain error. Reservations and committed authority are never part of the deletion set.

The two deletion phases make interruption recovery simple: staging and candidates are non-authoritative by definition, while the durable reservation continues to prevent identity reuse regardless of which deletion names reached stable storage.

## Plan drift examples

The integration suite explicitly proves that apply refuses to delete when:

- candidate bytes change after planning;
- a planned candidate is published as a new committed authority after planning;
- the operator omits `--confirm-abandoned`.

It also proves that unreserved higher artifacts are reported but retained, and that after successful cleanup of reserved generation 2 the next reservation is generation 3 rather than a reuse of generation 2.

## Platform boundary

Planning is read-only with respect to the generation namespace apart from the short-lived cooperative lock evidence and is available wherever generation-directory verification and the writer lease are supported.

Destructive application is currently Unix-only. Successful removal requires parent-directory durability barriers; non-Unix targets return unsupported instead of claiming an unproven deletion durability contract.

## Relationship to automatic cleanup

`db-lab-log-generation-cleanup` remains the no-confirmation routine maintenance command. It automatically removes only permanently obsolete history below current authority plus staging at/below authority.

This abandoned-artifact command is intentionally separate because higher uncommitted candidates may represent live work. Durable reservation makes their identity safe to reclaim, but human/operational abandonment confirmation remains a different proof obligation from storage correctness.

## Remaining Phase 1 boundary

This closes the repository-side identity-reuse obstacle to removing explicitly abandoned higher candidates/staging on Unix. It still does not provide:

- Windows-equivalent durable reservation/marker/deletion publication;
- automatic proof that a higher candidate is abandoned;
- legacy single-file append-log migration/coexistence;
- exclusion against non-cooperating raw-path writers or arbitrary external filesystem mutation.

The broad Phase 1 `Compaction` milestone therefore remains open.
