# Append-log compact-copy publication

`db-lab-log-compact` implements the first compaction primitive for the append-log engine without changing append-log on-disk format v1 and without replacing the authoritative source file.

The protocol identifier reported by the command is `append_log_compact_copy_v1`.

This is deliberately a **non-destructive compact copy**, not an in-place generation switch. The Phase 1 roadmap compaction item therefore remains incomplete until the engine has a crash-safe way to select a compacted generation as its authoritative path.

## Why the first primitive is non-destructive

The current append-log engine is one authoritative v1 file. Replacing that file through a portable `rename(temp, live)` sequence is not a sufficient crash protocol: overwrite semantics differ across platforms, a partially published target must never be mistaken for a valid recovered log, and a durable generation switch needs its own explicit contract.

Instead, the first primitive leaves the source untouched and publishes a **new path** only after a complete compact image has been produced and verified.

## Command

```text
db-lab-log-compact \
  --source data/history.db \
  --output data/history.compacted.db
```

Both paths are file paths. The source must already be a clean, fully verifiable append-log v1 file. The output and the deterministic sibling staging path must not exist.

For an output named `history.compacted.db`, the staging name is:

```text
.history.compacted.db.compacting
```

## State transformation

The command performs a read-only replay of the source with values included, then writes exactly one `put` record for every live key in bytewise key order. Deleted keys, overwritten historical versions, and tombstones are not copied.

The compacted file is still an ordinary append-log v1 file:

- the same 16-byte v1 file header is used;
- compacted records start at sequence 1 and remain contiguous;
- one live key becomes one put record;
- an empty logical database becomes a header-only valid v1 file;
- the result can immediately be opened, appended to, verified, inspected, and reopened by the existing engine.

Sequence numbers are local to the new compacted file. The compact copy preserves current logical state, not historical mutation identity.

## Fail-closed publication sequence

The command performs these steps:

1. require the source to be a real regular file rather than a symlink;
2. require the output and deterministic staging paths not to exist;
3. run read-only `LogEngine::verify` on the source;
4. reject a source with a recoverable incomplete final append instead of silently repairing it;
5. run read-only `LogEngine::inspect(..., true)` and require the verification report to match the initial verification;
6. create the staging file exclusively with `LogEngine::create_new`;
7. append one live-state put per inspected entry through the normal durable engine API;
8. inspect the staging file and require its complete live entries to equal the source live entries exactly, with no recoverable tail and exactly one record per live key;
9. inspect the source again and require it to be unchanged at the semantic/verification level during construction;
10. publish the complete staging inode to the fresh output path with `hard_link`, which fails rather than overwriting an existing target;
11. inspect the published output and require it to equal the verified staging image;
12. remove the staging name when possible and report whether an additional hard-link name had to be retained.

The source is never opened through mutable `LogEngine::open`, so the compaction command does not perform append-tail repair as a side effect.

## Crash and failure properties

The source remains authoritative throughout this protocol.

Before publication, a crash can leave only the hidden staging file. The requested output path does not exist, so callers cannot confuse a partially constructed compact image with a successfully published target.

After `hard_link` succeeds, the output name refers to the same already-built file object as the verified staging name. A crash before staging cleanup can therefore leave two names for one complete compact file, not a partial output. The source remains intact in either case.

If the filesystem does not support hard links for the requested sibling path, publication fails closed and the output is not created. General pre-publication failures perform best-effort staging cleanup. A pre-existing orphan staging path is never silently deleted because it may be evidence from an interrupted earlier attempt.

The engine is already documented as single-process/single-writer. This compact-copy primitive detects source changes across its verification/replay boundary, but it is not a multi-process lock protocol.

## Report

Successful stdout is JSON containing:

- `protocol = "append_log_compact_copy_v1"`;
- file format version;
- source byte and record counts;
- live-key count;
- compacted byte and record counts;
- bytes reclaimed in the compact copy;
- `staging_retained`, which is true only when output publication succeeded but unlinking the extra staging name failed.

`reclaimed_bytes` is descriptive physical-byte reduction between the clean source file and compact copy. It is not a performance claim.

## What remains before in-place compaction is complete

This primitive intentionally does **not** repoint an existing `LogEngine` path to the compact copy and does not delete the historical source. `docs/append-log-generation-switch.md` now freezes the executable recovery law for that later transition: the highest durably committed generation is authoritative, higher uncommitted files are ignored, and damage to the highest committed generation fails closed rather than silently rolling back to an older generation.

The remaining implementation must still assign concrete generation/marker bytes and names, bind each marker to the verified complete compacted base prefix, make marker publication durable across Linux/macOS/Windows, route future mutations to the selected generation, and adversarially test every filesystem interruption point against that recovery law.

Until that protocol exists and is tested adversarially, the roadmap's general append-log compaction milestone remains open.
