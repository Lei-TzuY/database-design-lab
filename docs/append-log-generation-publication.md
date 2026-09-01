# Append-log generation marker publication

`db-lab-log-generation-publish` is the standalone writer-side marker-v2 publication primitive for the current `append_log_generation_directory_v3` retained namespace.

```text
db-lab-log-generation-publish \
  --directory data/log-generations \
  --generation 2
```

The standalone command remains intentionally Unix-only and reports protocol `append_log_generation_marker_publication_unix_v1`. Windows standalone publication still fails before writing any marker. Windows marker authority is available only inside the composed compact-switch path, where an opaque in-process witness proves that the candidate's canonical generation name was just published by the audited write-through compact-output path.

## Common marker semantics

A final `commit-%020d.marker` is authoritative only through the shared generation-directory reader. Marker v2 binds:

- generation id;
- append-log format version;
- exact committed-prefix byte extent;
- CRC-32/IEEE of that prefix;
- committed record count;
- next sequence.

The exact bound prefix is independently re-verified through the normal append-log verifier before a marker can be accepted. Higher uncommitted generation logs, staging markers, and reservation files are never authoritative.

## Unix standalone durability order

The Unix standalone publisher requires a real clean canonical generation file, then:

1. derives and independently verifies the marker-bound committed prefix;
2. `sync_all`s the generation file;
3. `sync_all`s the generation directory so the generation name precedes marker authority durably;
4. re-verifies the exact generation and proof;
5. create-news `staging-commit-%020d.marker`, writes the 64-byte marker, and `sync_all`s it;
6. re-verifies the generation;
7. hard-links the synchronized staging inode to fresh `commit-%020d.marker`;
8. `sync_all`s the directory;
9. decodes the final marker and re-verifies its committed prefix;
10. removes the staging name best-effort.

The final marker is not reported as durably committed until the post-link directory barrier succeeds. No old generation is deleted by this command.

## Windows witness-bound compact-switch publication

Windows deliberately does **not** expose the standalone publisher. A clean-looking hand-created `generation-%020d.log` does not prove that its canonical namespace entry was durably published, so allowing an arbitrary caller to attach durable marker authority would widen the trust boundary incorrectly.

The Windows composed compact switch instead obtains an opaque `WindowsDurableCompactOutput` witness from `log_compaction`. That witness is constructible only after the compact candidate has completed all of the following in the same operation:

1. exact live-state construction and verification under a sibling staging name;
2. writable-handle `sync_all()` of the complete staging file;
3. no-overwrite `MoveFileExW(..., MOVEFILE_WRITE_THROUGH)` publication to the canonical generation name;
4. post-publication inspection equal to the verified staging image.

Only the crate-internal Windows marker path accepts that witness. It additionally requires the same generation id to have retained durable reservation evidence and requires the witness path to equal the canonical generation path in the verified directory.

The marker path then:

1. re-inspects the canonical generation and requires exact equality with the opaque compact witness;
2. derives and independently verifies marker-v2 committed-prefix proof;
3. create-news `staging-commit-%020d.marker`, writes the complete marker, and `sync_all`s it;
4. re-checks the compact witness immediately before authority changes;
5. moves staging to fresh final `commit-%020d.marker` with the audited no-overwrite `MOVEFILE_WRITE_THROUGH` primitive;
6. re-decodes the final marker, re-verifies the bound prefix, and re-checks the compact candidate;
7. lets the composed switch run the shared generation-directory verifier while still holding the common writer lease.

Successful Windows composed publication reports protocol `append_log_generation_marker_publication_windows_v1`. The write-through move consumes the staging marker, so successful summaries report `staging_retained=false`.

If the Win32 move reports an error after the final marker becomes visible, publication returns durability-uncertain and preserves retained evidence rather than guessing rollback. Hosted Windows CI exercises the API/order and retained-artifact semantics; it is not physical power-loss testing.

## Crash states and staging markers

`staging-commit-%020d.marker` is protocol-defined, non-authoritative crash residue. The v3 reader reports staging ids but never uses staging contents for authority selection. Zero-byte `reserve-%020d.frontier` files are also non-authoritative and permanently retire allocation ids.

On Unix, a visible final marker before the post-link directory barrier is explicitly durability-uncertain. On Windows composed switching, `MOVEFILE_WRITE_THROUGH` is the documented retained-name transition used for the final marker. In both cases, only final marker evidence can advance authority and the shared reader never falls back from a damaged highest committed generation.

## No-overwrite and monotonicity

Final marker targets are never overwritten. Unix uses `hard_link`; the Windows composed path uses `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` or `MOVEFILE_COPY_ALLOWED`. The requested generation must be newer than every existing final marker and, on Windows composed switching, must also have its retained durable reservation.

## Remaining lifecycle boundary

Unix already composes durable reservations, compact construction, cooperative writer exclusion, marker authority, routed mutation adoption, fault-matrix recovery, guarded cleanup/orphan retirement, and legacy migration/cutover.

Windows now has durable reservations, durable compact-candidate canonical-name publication, and witness-bound final marker authority in the composed compact-switch path. The standalone marker publisher intentionally remains unavailable on Windows.

The broad Phase 1 compaction milestone should remain open until the remaining Windows retained-entry lifecycle is filled in, particularly cleanup/orphan retirement and legacy migration/cutover. None of the hosted-CI checks are physical power-loss evidence.
