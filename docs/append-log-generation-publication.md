# Append-log generation marker publication

`db-lab-log-generation-publish` is the writer-side marker-v2 publication primitive for the current `append_log_generation_directory_v3` retained namespace.

```text
db-lab-log-generation-publish \
  --directory data/log-generations \
  --generation 2
```

The current publication protocol is `append_log_generation_marker_publication_unix_v1` and is intentionally available only on Unix targets. Unsupported targets, including Windows, fail before writing any artifact. This is deliberate: the repository does not claim a cross-platform parent-directory durability barrier that it cannot currently justify.

## Preconditions

The directory must be a real directory using the closed generation-directory namespace. The requested generation id must be greater than zero and newer than every existing committed marker id. Its canonical `generation-%020d.log` must be a real regular file and must pass `LogEngine::verify` as a completely clean append-log v1 image: no recoverable tail and `file_bytes == valid_bytes`.

The publisher derives the marker-v2 `CommittedPrefix` from that exact clean image: byte extent, CRC-32/IEEE, record count, and next sequence. It immediately re-verifies that proof through the shared committed-prefix verifier before publication can continue.

The tool assumes caller serialization. It actively re-verifies the generation after durability and staging work so concurrent mutation or replacement causes publication to fail instead of committing stale proof.

## Unix durability order

The successful path is intentionally ordered:

1. verify the requested generation as a clean append-log image;
2. derive and independently verify its marker-bound committed prefix;
3. `sync_all` the generation file;
4. `sync_all` the generation directory so the generation name precedes marker authority durably;
5. re-verify the exact generation and proof;
6. create-new `staging-commit-%020d.marker` in the same directory, write the 64-byte marker, and `sync_all` that staging file;
7. re-verify the generation again after staging I/O;
8. publish the final `commit-%020d.marker` with a no-overwrite hard link from the synchronized staging inode;
9. `sync_all` the generation directory again;
10. only after that directory barrier succeeds, decode the published marker and re-verify its committed prefix;
11. remove the staging name best-effort.

The final marker is not reported as durably committed until step 9 succeeds. No old generation is deleted by this command.

## Crash states and staging markers

`staging-commit-%020d.marker` is a protocol-defined, non-authoritative crash residue. The v3 directory reader accepts canonical staging names, reports their generation ids, and never uses their contents for generation selection. Directory v3 additionally accepts zero-byte `reserve-%020d.frontier` allocation evidence; reservations are also non-authoritative and are unrelated to marker selection.

This gives the publication sequence explicit crash behavior:

- Before the final hard link, a crash may leave a staging marker. It has no authority; the previously committed generation remains selected.
- After the final hard link but before the directory durability barrier completes, the final marker may be visible while persistence is uncertain. If the barrier returns an error, the publisher returns nonzero with a durability-uncertain error. The operator must preserve the old generation and use recovery/verification before any retry; the command does not pretend the marker is safely committed.
- After the final directory barrier succeeds, the final marker is the authoritative committed-generation evidence under the Unix publication contract. A leftover staging name is harmless and remains non-authoritative.
- Failure to remove staging after successful commit is reported as `staging_retained=true`; it does not revoke marker authority.

Stale canonical staging files are safe to remove before a retry only when they are real regular files. A symlink or non-file at the staging path fails closed.

## No-overwrite and monotonicity

Final markers are published with `hard_link`, not overwrite/rename replacement. The target marker must not already exist, and the requested generation must be newer than every existing committed marker id. A retry therefore cannot silently rewrite retained commit evidence.

Higher uncommitted generation logs, staging markers, and reservation files remain non-authoritative. Selection is still governed by the highest final `commit-%020d.marker`, as defined by `docs/append-log-generation-directory.md`.

## Windows boundary

The current binary fails closed on non-Unix targets before any filesystem mutation. In particular, the project does not equate flushing a marker file with durable publication of its parent-directory entry on Windows.

A future Windows implementation must establish and test an explicit directory-entry durability mechanism with equivalent crash semantics before the platform can be enabled. Until then, Windows CI validates the fail-before-write behavior rather than pretending to validate a durability guarantee that is not implemented.

## What this still does not complete

This primitive publishes commit authority for an already-created clean generation. The separate offline compact-switch operation now composes allocation, exact live-state construction, routed-writer exclusion, publication, and deterministic interruption checks across these publication boundaries. Directory v3 adds a durable reservation primitive, but the compact-switch allocator is not yet wired to create a reservation before candidate construction.

The broader lifecycle still lacks:

- Windows-equivalent durable marker/reservation publication;
- guarded cleanup of confirmed-abandoned higher generation/staging artifacts while retaining reservation evidence;
- migration/coexistence rules for legacy single-file `LogEngine` users.

Therefore the general Phase 1 compaction milestone remains open.
