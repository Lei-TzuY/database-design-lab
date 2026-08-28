from pathlib import Path

readme = Path("README.md")
text = readme.read_text()
text = text.replace(
    "The workspace currently contains six crates with executable behavior. The B+ tree is a complete common\npersistent point/range engine; the LSM work has begun with an explicitly bounded WAL/MemTable stage:",
    "The workspace currently contains six crates with executable behavior. The B+ tree is a complete common\npersistent point/range engine; the LSM now has a WAL/MemTable plus crash-published SSTable/manifest stage:",
)
text = text.replace(
    "| `db-storage-lsm` | Common persistent `KvEngine` foundation with its own versioned/checksummed WAL, synced PUT/tombstone records, deterministic recovery, ordered mutable/immutable MemTables, and half-open range scans; no SSTables or compaction yet |",
    "| `db-storage-lsm` | Common persistent `KvEngine` with its own checksummed WAL, ordered MemTables, indexed/checksummed immutable SSTables, immutable manifest snapshots, mirrored `CURRENT` publication, WAL-tail replay, tombstones, and half-open range scans; no Bloom filters, WAL reclamation, levels, or compaction yet |",
)
old = '''The LSM foundation is not an adapter around `db-storage-log`: it owns a distinct WAL format and keeps
sequence-tagged values/tombstones in an ordered mutable MemTable that freezes at a documented 64 KiB
resident estimate. Reads search immutable tables newest-first, ranges resolve the newest sequence per
key, and reopen deterministically reconstructs those table boundaries from the WAL. Because immutable
tables are not yet flushed to SSTables and the WAL is never reclaimed, this is correctness/recovery
evidence—not an LSM performance baseline and not yet a fair B+ tree comparison participant.'''
new = '''The LSM is not an adapter around `db-storage-log`: it owns a distinct WAL format and keeps
sequence-tagged values/tombstones in an ordered mutable MemTable that freezes at a documented 64 KiB
resident estimate. A frozen table is synchronously encoded as an indexed/checksummed immutable SSTable,
then referenced by a checksummed immutable manifest snapshot, and only then published through the
inactive slot of an 8 KiB mirrored `CURRENT` file. Reopen validates the selected manifest/SSTables and
replays only WAL sequences above the manifest's durable watermark. The WAL deliberately retains its
complete history in this slice, and there are no Bloom filters, levels, or compaction, so this remains
correctness/recovery evidence—not yet a fair B+ tree performance comparison participant.'''
if old not in text:
    raise SystemExit("README LSM foundation paragraph changed")
text = text.replace(old, new, 1)
text = text.replace(
    "multi-process writers, LSM SSTables/compaction, replication, SQL, MVCC, Raft, graph, time-series, and\ncolumnar execution are not implemented.",
    "multi-process writers, LSM Bloom filters/levels/compaction/WAL reclamation, replication, SQL, MVCC,\nRaft, graph, time-series, and columnar execution are not implemented.",
)
readme.write_text(text)

roadmap = Path("docs/roadmap.md")
text = roadmap.read_text()
old = '''- [ ] Complete the WAL/SSTable/manifest format family and atomic version-set transitions. WAL v1 and
  its single-file directory policy are specified in `docs/lsm-wal-format.md`; SSTable, manifest,
  CURRENT, flush-install, and WAL-reclamation formats remain deliberately unspecified until their
  implementation PR.'''
new = '''- [x] Specify and implement WAL/SSTable/manifest persistence through atomic flush installation. WAL v1
  remains in `docs/lsm-wal-format.md`; `docs/lsm-sstable-manifest-format.md` defines indexed/checksummed
  immutable SSTables, complete immutable manifest snapshots, mirrored 4 KiB `CURRENT` slots, durable
  sequence watermarks, canonical orphan handling, and the WAL-backed interrupted-install recovery rule.
- [ ] Specify and implement WAL segment rotation/reclamation. The current single WAL intentionally retains
  complete history so reclamation does not share the first SSTable/manifest publication protocol.'''
if old not in text:
    raise SystemExit("roadmap persistence item changed")
text = text.replace(old, new, 1)
text = text.replace(
    "- [ ] Implement immutable sorted tables with indexes and checksums.",
    "- [x] Implement immutable sorted tables with complete indexes, per-record/index/header/footer checksums,\n  whole-file checksum validation, full 4 KiB-key/1 MiB-value support, and manifest-bound key/extent metadata.",
    1,
)
text = text.replace(
    "- [ ] Implement tombstone-aware reads, levels, compaction, and crash-safe manifest recovery.",
    "- [x] Implement tombstone-aware multi-SSTable point/range reads plus crash-safe flush manifest recovery.\n  Tests cover authoritative SSTable/manifest corruption, latest-CURRENT-slot fallback with WAL replay,\n  unreferenced canonical orphans, mutable WAL tails, and maximum-value flush/reopen.\n- [ ] Add levels, overlap policy, compaction, obsolete-file deletion, and safe tombstone dropping.",
    1,
)
roadmap.write_text(text)

wal = Path("docs/lsm-wal-format.md")
text = wal.read_text()
old = '''This document specifies only the Phase 3 behavior implemented today: one checksummed write-ahead log,
one ordered mutable MemTable, and zero or more ordered immutable MemTables reconstructed in memory.
There are no SSTables, manifests, Bloom filters, levels, compaction, background flushes, or WAL
reclamation yet. Consequently this engine is executable correctness evidence, but not yet a candidate
for B+ tree versus LSM performance claims.'''
new = '''This document specifies the WAL and in-memory half of Phase 3. The engine now also has persistent
indexed SSTables, immutable manifest snapshots, mirrored `CURRENT`, and synchronous flush publication;
those bytes and crash states are specified separately in `docs/lsm-sstable-manifest-format.md`.
Bloom filters, WAL rotation/reclamation, levels, and compaction remain deferred. Consequently the engine
is executable correctness evidence, but not yet a candidate for B+ tree versus LSM performance claims.'''
if old not in text:
    raise SystemExit("WAL intro changed")
text = text.replace(old, new, 1)
old_dir = '''An engine path is a directory containing exactly one regular file:

```text
wal-0000000000000001.log
```

Opening an existing empty directory, an unknown entry, a non-regular WAL entry, or a missing WAL fails
closed. `create_new` atomically reserves the directory name and rejects every existing path. If initial
WAL creation fails, the reserved but incomplete directory remains invalid rather than being guessed at
or silently reinitialized. The implementation synchronizes the initial WAL contents, but does not use a
platform-specific parent-directory sync; a system crash immediately after creation may therefore lose
the directory entry.

The fixed file name and WAL id deliberately leave no implied segment-rotation protocol. WAL numbering,
SSTable naming, temporary-file handling, and manifest publication will be specified together when real
flush/install behavior exists.'''
new_dir = '''The active WAL remains the fixed regular file:

```text
wal-0000000000000001.log
```

New engines also contain `CURRENT` and at least one immutable manifest snapshot; published flushes add
canonical SSTable/manifest files as specified in `docs/lsm-sstable-manifest-format.md`. An exact legacy
WAL-only directory from the earlier Phase 3 slice remains readable and replays from sequence one; partial
version-set layouts do not receive that compatibility treatment. Unknown entries and non-regular files
still fail closed. `create_new` reserves the directory name and rejects every existing path. The
implementation synchronizes file contents but does not use a portable parent-directory sync protocol.

The fixed WAL file name/id still deliberately defines no segment rotation or reclamation protocol. The
single WAL retains all complete mutation records even after their sequences are represented by published
SSTables.'''
if old_dir not in text:
    raise SystemExit("WAL directory section changed")
text = text.replace(old_dir, new_dir, 1)
text = text.replace(
    '''Immutable here means no later mutation changes that in-memory table. It does **not** mean persisted:
every table still depends on the unreclaimed WAL after restart. There is no background worker or
concurrent mutation contract; one caller and one process must serialize access.''',
    '''Immutable here means no later mutation changes that in-memory table. Frozen tables are now flushed
synchronously and retired only after SSTable + manifest + CURRENT publication succeeds. The unreclaimed
WAL remains a redundant recovery source and supplies every sequence above the selected manifest durable
watermark. There is no background worker or concurrent mutation contract; one caller and one process
must serialize access.''',
    1,
)
old_deferred = '''The following require another focused design plus executable evidence: immutable SSTable bytes and
indexes, block checksums, manifest/CURRENT formats, atomic version-set installation, flush crash states,
WAL rotation/reclamation, Bloom filters, levels, tombstone dropping rules, compaction, read/write/space
amplification instrumentation, and performance comparisons. Unknown directory entries are rejected
precisely because none of those formats has been declared yet.'''
new_deferred = '''The following still require focused design plus executable evidence: WAL segment rotation/reclamation,
Bloom filters, block/cache layout, levels and overlap policy, tombstone dropping rules, compaction and
obsolete-file deletion, compaction fault injection, read/write/space amplification instrumentation, and
performance comparisons. SSTable/manifest/CURRENT bytes and first-stage flush crash states are now
specified in `docs/lsm-sstable-manifest-format.md`.'''
if old_deferred not in text:
    raise SystemExit("WAL deferred section changed")
text = text.replace(old_deferred, new_deferred, 1)
wal.write_text(text)
