from pathlib import Path


def replace(path: Path, old: str, new: str, count: int = 1) -> None:
    text = path.read_text()
    if text.count(old) < count:
        raise SystemExit(f"missing expected text in {path}: {old[:180]!r}")
    path.write_text(text.replace(old, new, count))

readme = Path("README.md")
replace(
    readme,
    "| `db-storage-lsm` | Common persistent `KvEngine` with its own checksummed WAL, ordered MemTables, indexed/checksummed immutable SSTables, immutable manifest snapshots, mirrored `CURRENT` publication, WAL-tail replay, tombstones, and half-open range scans; no Bloom filters, WAL reclamation, levels, or compaction yet |",
    "| `db-storage-lsm` | Common persistent `KvEngine` with checksummed segmented WALs, ordered MemTables, indexed/checksummed immutable SSTables, immutable manifest snapshots, mirrored `CURRENT` publication, crash-safe WAL rotation/reclamation, tombstones, and half-open range scans; no Bloom filters, levels, or compaction yet |",
)
replace(
    readme,
    '''The LSM is not an adapter around `db-storage-log`: it owns a distinct WAL format and keeps
sequence-tagged values/tombstones in an ordered mutable MemTable that freezes at a documented 64 KiB
resident estimate. A frozen table is synchronously encoded as an indexed/checksummed immutable SSTable,
then referenced by a checksummed immutable manifest snapshot, and only then published through the
inactive slot of an 8 KiB mirrored `CURRENT` file. Reopen validates the selected manifest/SSTables and
replays only WAL sequences above the manifest's durable watermark. The WAL deliberately retains its
complete history in this slice, and there are no Bloom filters, levels, or compaction, so this remains
correctness/recovery evidence—not yet a fair B+ tree performance comparison participant.''',
    '''The LSM is not an adapter around `db-storage-log`: it owns a distinct WAL format and keeps
sequence-tagged values/tombstones in an ordered mutable MemTable that freezes at a documented 64 KiB
resident estimate. A frozen table is synchronously encoded as an indexed/checksummed immutable SSTable,
then referenced by a checksummed immutable manifest snapshot, and only then published through the
inactive slot of an 8 KiB mirrored `CURRENT` file. Manifest v2 also binds the authoritative WAL segment
id and first sequence. When the published SSTable watermark reaches the active WAL tail, the engine
creates and synchronizes a new empty WAL, publishes a new manifest that names it, mirrors that same
manifest into the other `CURRENT` slot, and only then removes obsolete WAL segments. Reopen therefore
needs only the manifest-selected WAL suffix while both CURRENT mirrors remain valid after reclamation.
There are still no Bloom filters, levels, or compaction, so this remains correctness/recovery evidence—not
yet a fair B+ tree performance comparison participant.''',
)
replace(
    readme,
    "multi-process writers, LSM Bloom filters/levels/compaction/WAL reclamation, replication, SQL, MVCC,\nRaft, graph, time-series, and columnar execution are not implemented.",
    "multi-process writers, LSM Bloom filters/levels/compaction, replication, SQL, MVCC, Raft, graph,\ntime-series, and columnar execution are not implemented.",
)
replace(
    readme,
    '''The LSM foundation stores its own WAL in an engine directory. Its header and each PUT/DELETE record
carry independent magic/version fields, bounded little-endian lengths, contiguous sequence numbers,
and header/full-record CRC-32 checksums. `write_all` and `sync_data` complete before a mutation enters
the MemTable or returns. Reopen replays complete records and truncates only a structurally canonical
incomplete final record; unknown directory entries, invalid headers, sequence gaps, absurd lengths,
unexplained tails, and complete checksum failures fail closed. No SSTable or manifest commit guarantee
is claimed because those formats do not exist yet.''',
    '''The LSM stores each WAL segment as `wal-%016d.log`; its checksummed header binds the segment id and
first sequence, while every PUT/DELETE record retains contiguous global sequence numbers and independent
header/full-record CRC-32 checksums. `write_all` and `sync_data` complete before a mutation enters the
MemTable or returns. Reopen selects the WAL named by the authoritative manifest, validates its header
identity, replays complete records above the manifest durable watermark, and truncates only a structurally
canonical incomplete final record. Frozen MemTables become synchronized immutable SSTables before a new
immutable manifest is published through mirrored `CURRENT`. WAL reclamation is a second publication
step: a new empty segment is synchronized, Manifest v2 names its id/first sequence, both CURRENT mirrors
are moved to that same manifest, the old WAL handle is closed, and only then are obsolete canonical WAL
segments removed. Unknown entries, identity mismatches, sequence gaps, absurd lengths, unexplained tails,
and complete checksum failures fail closed.''',
)
replace(
    readme,
    "[the LSM WAL/MemTable format](docs/lsm-wal-format.md), and\n[the experimental constitution]",
    "[the LSM WAL/MemTable format](docs/lsm-wal-format.md),\n[the LSM SSTable/manifest format](docs/lsm-sstable-manifest-format.md), and\n[the experimental constitution]",
)
replace(
    readme,
    '''- [LSM WAL/MemTable foundation](docs/lsm-wal-format.md): directory/WAL bytes, replay and tail policy,
  deterministic MemTable freezing, and explicitly deferred SSTable/manifest behavior.''',
    '''- [LSM WAL/MemTable format](docs/lsm-wal-format.md): segmented WAL bytes, sequence identity, replay/tail
  recovery, deterministic MemTable freezing, and reclamation boundary.
- [LSM SSTable/manifest format](docs/lsm-sstable-manifest-format.md): immutable sorted tables, Manifest v2
  WAL binding, mirrored CURRENT publication, WAL rotation/reclamation, and recovery states.''',
)

roadmap = Path("docs/roadmap.md")
replace(
    roadmap,
    '''- [ ] Specify and implement WAL segment rotation/reclamation. The current single WAL intentionally retains
  complete history so reclamation does not share the first SSTable/manifest publication protocol.''',
    '''- [x] Specify and implement crash-safe WAL segment rotation/reclamation. Manifest v2 binds an active WAL
  id and first sequence; rotation occurs only when the SSTable durable watermark reaches that WAL's tail.
  A new segment is synchronized before publication, the same new manifest is installed into both CURRENT
  mirrors before old WAL deletion, legacy Manifest v1 remains readable, and canonical orphan WAL ids are
  skipped rather than overwritten.''',
)

wal = Path("docs/lsm-wal-format.md")
replace(
    wal,
    '''This document specifies the WAL and in-memory half of Phase 3. The engine now also has persistent
indexed SSTables, immutable manifest snapshots, mirrored `CURRENT`, and synchronous flush publication;
those bytes and crash states are specified separately in `docs/lsm-sstable-manifest-format.md`.
Bloom filters, WAL rotation/reclamation, levels, and compaction remain deferred. Consequently the engine
is executable correctness evidence, but not yet a candidate for B+ tree versus LSM performance claims.''',
    '''This document specifies the WAL and in-memory half of Phase 3. The WAL byte format remains version 1,
but WALs are now canonical numbered segments selected by Manifest v2. Indexed SSTables, immutable
manifest snapshots, mirrored `CURRENT`, flush publication, and the cross-file rotation/reclamation
protocol are specified with `docs/lsm-sstable-manifest-format.md`. Bloom filters, levels, and compaction
remain deferred. Consequently the engine is executable correctness evidence, but not yet a candidate for
B+ tree versus LSM performance claims.''',
)
start = '''The active WAL remains the fixed regular file:

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
new = '''WAL files use the canonical name `wal-%016d.log`. A new engine starts with WAL id 1 and first sequence
1; later rotations allocate ids above every canonical WAL id observed in the directory, including
unreferenced crash orphans. Manifest v2 identifies exactly one authoritative WAL by id and first sequence.
Canonical non-authoritative WALs may exist after an interrupted rotation and are ignored for replay while
still reserving their numeric ids.

New engines also contain `CURRENT` and an immutable manifest snapshot; published flushes add canonical
SSTable/manifest files as specified in `docs/lsm-sstable-manifest-format.md`. An exact legacy WAL-only
directory containing only `wal-0000000000000001.log` remains readable with implicit WAL id/first sequence
1. Partial version-set layouts do not receive that compatibility treatment. Unknown entries and
non-regular files still fail closed. `create_new` reserves the directory name and rejects every existing
path. File contents are synchronized, but there is still no portable parent-directory fsync claim.'''
replace(wal, start, new)
replace(wal, "| 16 | 8 | WAL id | `1`, matching the fixed file name |", "| 16 | 8 | WAL id | nonzero and exactly the manifest-selected/canonical filename id |")
replace(wal, "| 24 | 8 | first sequence | `1` |", "| 24 | 8 | first sequence | nonzero and exactly the manifest-selected first sequence |")
replace(wal, "Each complete mutation consists of a 32-byte header followed immediately by `key || value`. DELETE has\nno value bytes. Sequence numbers are contiguous and start at one.", "Each complete mutation consists of a 32-byte header followed immediately by `key || value`. DELETE has\nno value bytes. Sequence numbers are globally contiguous within a segment beginning at the first sequence\ndeclared by its validated WAL header/manifest binding.")
replace(
    wal,
    '''Immutable here means no later mutation changes that in-memory table. Frozen tables are now flushed
synchronously and retired only after SSTable + manifest + CURRENT publication succeeds. The unreclaimed
WAL remains a redundant recovery source and supplies every sequence above the selected manifest durable
watermark. There is no background worker or concurrent mutation contract; one caller and one process
must serialize access.''',
    '''Immutable here means no later mutation changes that in-memory table. Frozen tables are flushed
synchronously and retired only after SSTable + manifest + CURRENT publication succeeds. If a newer
mutable suffix still exists in the active WAL, that WAL remains authoritative and cannot be reclaimed.
Only when the manifest durable watermark reaches the segment tail may rotation publish a new empty WAL
at `durable_sequence + 1`; reclamation then waits until both CURRENT mirrors name the new manifest. There
is no background worker or concurrent mutation contract; one caller and one process must serialize access.''',
)
replace(
    wal,
    '''The following still require focused design plus executable evidence: WAL segment rotation/reclamation,
Bloom filters, block/cache layout, levels and overlap policy, tombstone dropping rules, compaction and
obsolete-file deletion, compaction fault injection, read/write/space amplification instrumentation, and
performance comparisons. SSTable/manifest/CURRENT bytes and first-stage flush crash states are now
specified in `docs/lsm-sstable-manifest-format.md`.''',
    '''The following still require focused design plus executable evidence: Bloom filters, block/cache layout,
levels and overlap policy, tombstone dropping rules, compaction and obsolete-SSTable/manifest deletion,
compaction fault injection, read/write/space amplification instrumentation, and performance comparisons.
SSTable/Manifest v2/CURRENT bytes, flush crash states, and WAL rotation/reclamation are specified in
`docs/lsm-sstable-manifest-format.md`.''',
)

manifest = Path("docs/lsm-sstable-manifest-format.md")
replace(manifest, "# LSM SSTable and manifest publication v1", "# LSM SSTable, manifest, and WAL publication v2")
replace(
    manifest,
    '''This document specifies the first persistent sorted-table slice of Phase 3. Mutations are still
acknowledged by the existing single checksummed WAL before entering memory. A frozen MemTable is now
installed as an immutable indexed SSTable through an immutable manifest snapshot and a mirrored
`CURRENT` publication point. Bloom filters, levels, compaction, tombstone dropping, WAL segmentation,
and WAL reclamation are deliberately outside this version.''',
    '''This document specifies persistent sorted-table publication plus crash-safe WAL segmentation and
reclamation. Mutations are acknowledged by a checksummed WAL segment before entering memory. A frozen
MemTable is installed as an immutable indexed SSTable through an immutable manifest snapshot and mirrored
`CURRENT`; when that durable watermark reaches the active WAL tail, Manifest v2 can atomically switch the
version set to a new empty WAL and reclaim older segments only after both CURRENT mirrors move. Bloom
filters, levels, compaction, and tombstone dropping remain outside this version.''',
)
replace(
    manifest,
    '''`CURRENT` determines the authoritative manifest. Canonically named SSTables or manifests not reachable
through the selected `CURRENT` slot are orphans from an interrupted or superseded install and are not
opened as authoritative data. They are nevertheless included when choosing the next numeric id, so a
later flush never overwrites an orphan whose durability outcome was ambiguous. Unknown names and
non-regular entries fail closed.''',
    '''`CURRENT` determines the authoritative manifest. That manifest determines the authoritative WAL segment.
Canonically named SSTables, manifests, or WALs not selected by this chain are orphans from interrupted or
superseded publication and are not opened as authoritative state. Their ids still reserve numeric space,
so later allocation never overwrites a file whose durability outcome was ambiguous. Unknown names and
non-regular entries fail closed.''',
)
replace(
    manifest,
    '''A `MANIFEST-%016d` is a complete version-set snapshot, not an append log. Its checksummed header stores
manifest id, the version set's durable WAL sequence, table count, and descriptor-body length. Every
SSTable descriptor is individually checksummed, and the entire manifest has a trailing CRC.''',
    '''A `MANIFEST-%016d` is a complete version-set snapshot, not an append log. Manifest v1 uses its original
64-byte header and remains readable with implicit WAL id 1 / first sequence 1. Newly written Manifest v2
uses an 80-byte header that stores manifest id, durable sequence, table count, descriptor-body length,
a nonzero authoritative WAL id, and that WAL's nonzero first sequence, followed by reserved zeros and a
header CRC. Every SSTable descriptor is individually checksummed, and the entire manifest has a trailing
CRC.''',
)
old_retention = '''## WAL retention and reads

This slice intentionally does **not** reclaim the WAL. The active WAL continues to contain every mutation
from sequence 1 onward. `durable_sequence` changes only replay application: records at or below the
selected manifest watermark are validated while scanning the WAL but are not re-applied to MemTables;
records above it rebuild the unflushed tail.

Point reads search mutable/frozen MemTables first and then authoritative SSTables newest-first. Ordered
range scans merge sequence-tagged SSTable state and the in-memory tail, keep the newest version of each
key, remove tombstones, and apply the common half-open bounds/limit. Compaction is required before old
table versions or tombstones can be discarded safely.'''
new_retention = '''## WAL rotation, mirror safety, and reads

Rotation is eligible only when the active WAL contains at least one record and its `next_sequence` is
exactly `durable_sequence + 1`; this proves no unflushed mutable suffix still depends on that segment. The
engine then creates and synchronizes a new empty canonical WAL whose first sequence is
`durable_sequence + 1`, writes/synchronizes a new Manifest v2 naming the new WAL and unchanged SSTable
set, and publishes that manifest through the next CURRENT generation.

At this point the older CURRENT mirror may still select the previous manifest/WAL, so the old WAL is not
yet reclaimable. The engine writes the **same new manifest id** into the other CURRENT slot at the next
generation and synchronizes it. Only after both valid mirrors select the new manifest does it swap/drop
the old WAL handle and best-effort remove every other canonical WAL segment. This ordering is also
Windows-safe because no obsolete WAL is removed while its live file handle is retained.

A crash before the first CURRENT update leaves the old manifest/WAL authoritative and the new files as
canonical orphans. A crash after the first CURRENT update but before mirror completion can select either
version, so both WALs remain present. After mirror completion, either CURRENT slot selects the same new
manifest/WAL and old segments are redundant. A replayed frozen MemTable followed by a newer mutable tail
specifically cannot trigger reclamation until later flushes advance the durable watermark through that
tail.

Point reads search mutable/frozen MemTables first and then authoritative SSTables newest-first. Ordered
range scans merge sequence-tagged SSTable state and the active WAL/MemTable tail, keep the newest version
of each key, remove tombstones, and apply the common half-open bounds/limit. Compaction is still required
before old SSTable versions or tombstones can be discarded safely.'''
replace(manifest, old_retention, new_retention)
replace(
    manifest,
    '''The next format/protocol work is WAL segment rotation and reclamation. It must not delete a WAL prefix
until a published version set proves the corresponding sequences are durable and recovery remains
unambiguous. Bloom filters, leveled placement, overlap rules, compaction selection, crash-safe compaction
publication, obsolete-file deletion, tombstone elision, block/cache design, and amplification
instrumentation remain separate evidence milestones.''',
    '''Bloom filters, leveled placement, overlap rules, compaction selection, crash-safe compaction publication,
obsolete SSTable/manifest deletion, tombstone elision, block/cache design, and amplification
instrumentation remain separate evidence milestones. WAL segment rotation/reclamation is now part of the
implemented crash-state protocol; parent-directory durability remains subject to the repository-wide
portable-fsync caveat.''',
)
