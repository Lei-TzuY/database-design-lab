# Append-log generation orphan retirement

`db-lab-log-generation-orphan` provides an explicit inspect/retire workflow for higher uncommitted generation candidates that an operator has independently determined are abandoned.

```text
db-lab-log-generation-orphan inspect \
  --directory data/log-generations \
  --generation 17

db-lab-log-generation-orphan retire \
  --directory data/log-generations \
  --generation 17 \
  --expected-authority 12 \
  --expected-bytes 1048576 \
  --expected-crc32 1234567890 \
  --confirm-generation-builder-stopped
```

Inspection protocol is `append_log_generation_orphan_inspect_v1`. Unix retirement protocol is `append_log_generation_orphan_retire_unix_v1`.

## Why retirement retains a staging name

Generation ids are monotonic. The shared allocator advances above every observed generation log, final marker, and staging-marker id. Simply deleting a higher orphan generation would erase the only evidence that its id had already been observed and could permit a later allocator to reuse a lower id.

Retirement therefore does not erase the generation id. It replaces the large orphan log with durable, non-authoritative frontier evidence using the already-defined `staging-commit-%020d.marker` namespace. Reader recovery never selects or decodes staging-marker contents, while the allocator continues to count their ids.

This deliberately avoids a new generation-directory format or mutable high-watermark file. When current authority eventually advances to or above the retained staging id, normal obsolete-history cleanup may remove that staging name because the committed authority then preserves an equal-or-higher frontier.

## Guarded operator workflow

`inspect` is read-only. It requires a real canonical uncommitted generation above current authority and returns:

- current authoritative generation;
- orphan generation and filename;
- exact byte length;
- streaming CRC-32/IEEE fingerprint;
- whether a same-id staging frontier already exists;
- current highest observed generation.

The fingerprint is an accidental-drift guard, not a cryptographic authorship or adversarial-integrity proof.

`retire` requires the exact inspected authority, byte length, and CRC plus `--confirm-generation-builder-stopped`. The tool never infers builder liveness from PID, age, modification time, or filename. That explicit confirmation is load-bearing because compact candidate construction intentionally occurs outside the cooperative writer lease.

## Unix retirement order

After confirmation, retirement:

1. acquires the shared generation writer lease;
2. verifies the expected current authority;
3. confirms the requested generation is higher, canonical, uncommitted, real, and fingerprint-identical to the inspected candidate;
4. ensures a same-id staging frontier exists as a real regular file; if absent, creates a small fixed sentinel with create-new semantics and synchronizes it; if already present, preserves its bytes and synchronizes the existing file;
5. synchronizes the generation directory so the frontier name is durable before reclamation;
6. re-verifies authority, orphan status, staging frontier, and exact orphan fingerprint;
7. removes the orphan generation log;
8. synchronizes the generation directory again;
9. re-verifies unchanged authority, absence of the retired log, presence of the staging frontier, and a highest-observed generation at least as large as the retired id.

If the frontier directory barrier fails, the orphan is not deleted. If the post-removal directory barrier fails, retirement reports durability-uncertain rather than success; the frontier had already been durably established first, so id reuse remains prevented.

## Existing frontier evidence

A pre-existing same-id staging marker is never overwritten. Retirement only verifies it is a real regular file and synchronizes it before deleting the orphan. Its contents remain opaque and non-authoritative under the existing reader contract.

## Platform and concurrency boundary

Retirement is Unix-only because it relies on parent-directory durability after creating the frontier name and after removing the orphan log. Non-Unix retirement fails before filesystem access.

The shared writer lease excludes coordinated routed mutations, marker publication, cleanup, and compact-switch publication while retirement runs. It cannot stop a non-cooperating process that is still building the candidate outside that lease. The explicit builder-stopped confirmation therefore remains mandatory even though the candidate fingerprint is rechecked immediately before deletion.

## What this does not do

This protocol does not automatically decide that a candidate is abandoned, remove retained frontier evidence above current authority, provide Windows marker/deletion durability, migrate legacy single-file logs, or protect against direct raw-path writers that bypass the generation lifecycle contract.
