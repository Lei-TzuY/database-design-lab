from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        raise SystemExit(f"missing docs marker: {label}")
    return text.replace(old, new, 1)


# README
p = Path("README.md")
text = p.read_text()
text = replace_once(
    text,
    "| `db-storage-lsm` | Common persistent `KvEngine` with checksummed segmented WALs, ordered MemTables, indexed/checksummed immutable SSTables, validated embedded Bloom filters, Manifest-v4 L0/L1 and tombstone-GC metadata, crash-published full-set compaction, mirrored `CURRENT`, WAL/SSTable/manifest reclamation, and half-open range scans |",
    "| `db-storage-lsm` | Common persistent `KvEngine` with checksummed segmented WALs, ordered MemTables, indexed/checksummed immutable SSTables, validated embedded Bloom filters, Manifest-v5 L0/L1, tombstone-GC, and SSTable-allocation metadata, crash-published full-set compaction, mirrored `CURRENT`, WAL/SSTable/manifest reclamation, and half-open range scans |",
    "README engine table",
)
old = '''full-set merge of all authoritative SSTables into at most one L1 run. Because the current engine is
caller-serialized, has no snapshots, and consumes every older disk run, it may elide a newest tombstone
at that exact proof point. Manifest v4 records the resulting `tombstone_gc_sequence`; if no live keys
remain, it safely carries a nonzero durable watermark with zero SSTables. The optional compacted SSTable
and manifest are synchronized, the same manifest is published through both CURRENT mirrors, and only
then are obsolete files eligible for best-effort deletion.'''
new = '''full-set merge of all authoritative SSTables into at most one L1 run. Because the current engine is
caller-serialized, has no snapshots, and consumes every older disk run, it may elide a newest tombstone
at that exact proof point. Manifest v4 introduced the resulting `tombstone_gc_sequence`; Manifest v5
preserves that field and additionally persists the highest SSTable id that has been allocated or observed
under a canonical name. A fully deleted database can therefore carry a nonzero durable watermark with
zero SSTables without forgetting ids whose durability was previously ambiguous. The optional compacted
SSTable and manifest are synchronized, the same manifest is published through both CURRENT mirrors, and
only then are obsolete files eligible for best-effort deletion.'''
text = replace_once(text, old, new, "README LSM v5 paragraph")
p.write_text(text)


# Main LSM format specification
p = Path("docs/lsm-sstable-manifest-format.md")
text = p.read_text()
text = replace_once(text, "# LSM SSTable, manifest, WAL, and L0/L1 compaction v4", "# LSM SSTable, manifest, WAL, and L0/L1 compaction v5", "format title")
old = '''SSTable v2 embeds a validated Bloom filter. Manifest v3 added each table's level and a
correctness-first overlapping-L0 / single-run-L1 compaction policy. Manifest v4 adds an explicit
tombstone-GC watermark so a full-set compaction can discard deletion markers without losing the durable
sequence when the live result contains no SSTable entries.'''
new = '''SSTable v2 embeds a validated Bloom filter. Manifest v3 added each table's level and a
correctness-first overlapping-L0 / single-run-L1 compaction policy. Manifest v4 added an explicit
tombstone-GC watermark so a full-set compaction can discard deletion markers without losing the durable
sequence when the live result contains no SSTable entries. Manifest v5 keeps that GC field and adds a
persistent SSTable-id high watermark so table-less cleanup cannot make a historical or crash-ambiguous
canonical id eligible for reuse.'''
text = replace_once(text, old, new, "format intro")
old = '''Canonically named SSTables, manifests, or WALs not selected by this chain are orphans from interrupted or
superseded publication and are not opened as authoritative state. Their ids still reserve numeric space,
so later allocation never overwrites a file whose durability outcome was ambiguous. Unknown names and
non-regular entries fail closed.'''
new = '''Canonically named SSTables, manifests, or WALs not selected by this chain are orphans from interrupted or
superseded publication and are not opened as authoritative state. Their ids still reserve numeric space,
so later allocation never overwrites a file whose durability outcome was ambiguous. On open, every
canonical SSTable filename raises the in-memory allocation floor even when the file is not authoritative;
the next Manifest v5 publication persists that observed high watermark before post-publication cleanup
may remove the orphan name. Unknown names and non-regular entries fail closed.'''
text = replace_once(text, old, new, "directory allocation floor")
start = text.index("A `MANIFEST-%016d` is a complete version-set snapshot, not an append log.")
end = text.index("\n\n## Level policy and full-set compaction", start)
manifest_section = '''A `MANIFEST-%016d` is a complete version-set snapshot, not an append log. Manifest v1 uses its original
64-byte header and remains readable with implicit WAL id 1 / first sequence 1. Manifest v2 introduced an
80-byte header containing manifest id, durable sequence, table count, descriptor-body length, authoritative
WAL id, WAL first sequence, reserved zeros, and header CRC. Manifest v3 keeps that 80-byte header and
changes only the SSTable descriptor body. Manifest v4 also stays 80 bytes: bytes 64–71 are
`tombstone_gc_sequence`, bytes 72–75 are zero, and bytes 76–79 are the header CRC.

Manifest v5 expands the header to 88 bytes while preserving every v4 field in place. Bytes 64–71 remain
`tombstone_gc_sequence`; bytes 72–79 are `table_id_high_watermark`; bytes 80–83 are zero; bytes 84–87
are the header CRC over bytes 0–83. The descriptor body therefore begins at offset 88 for v5 and at its
historical offset for every older version. V1–v3 imply a GC watermark of zero. V1–v4 do not encode an
allocation high watermark: a nonempty legacy manifest derives it from the largest active descriptor id,
while a table-less legacy manifest with durable history conservatively reserves ids through its durable
sequence. This may skip unused numbers, but it cannot reconstruct and therefore must not guess a smaller
historical allocation frontier. Every newly written manifest is v5.

Manifest v1/v2 descriptors have a 40-byte prefix and are interpreted as level 0 for compatibility.
Manifest v3/v4/v5 descriptors use a 48-byte prefix: table id, file bytes, entry count, durable sequence,
a u32 level, a zero u32 reserved field, smallest/largest-key lengths, key bounds, and descriptor CRC. The
implemented policy accepts only levels 0 and 1 and at most one L1 descriptor. Descriptors remain ordered
by strictly increasing table id and durable sequence. With a nonempty table set, the manifest-level
durable sequence must equal the newest descriptor's watermark. A table-less v4/v5 version may retain a
nonzero durable sequence only when `tombstone_gc_sequence == durable_sequence`; this records that a
complete compaction consumed every older physical version instead of pretending absent table bytes cover
live values. The GC watermark may never exceed the durable sequence, and an install may move neither
the durable nor GC watermark backward.

For v5, durable history requires a nonzero `table_id_high_watermark`, and every active descriptor id must
be at or below it. `open` additionally raises the in-memory watermark to the largest canonical SSTable id
observed in the directory, including unreferenced crash orphans. Flush, compaction, or WAL rotation then
carries the maximum forward into the next immutable v5 snapshot. In particular, table-less compaction may
clean up all SSTable filenames only after CURRENT publishes a manifest that remembers their allocation
frontier. No manifest is accepted merely because the WAL could reconstruct its data: a manifest selected
by CURRENT is authoritative persistent state, so checksum, level, descriptor, GC-watermark,
allocation-watermark, or empty-version invariant failure fails closed.'''
text = text[:start] + manifest_section + text[end:]
text = replace_once(
    text,
    "4. The replacement Manifest v4 sets `tombstone_gc_sequence = durable_sequence`. Future flushes preserve\n   that watermark, and a later full-set compaction may advance it.",
    "4. The replacement Manifest v5 sets `tombstone_gc_sequence = durable_sequence` and preserves an\n   SSTable-id high watermark at least as large as every active or canonically observed table id. Future\n   flushes preserve both monotonic watermarks, and a later full-set compaction may advance them.",
    "compaction proof point",
)
text = text.replace("Create and `sync_all` a new Manifest v4", "Create and `sync_all` a new Manifest v5")
text = text.replace("replacement L1\nSSTable, Manifest v4, first CURRENT slot", "replacement L1\nSSTable, Manifest v5, first CURRENT slot")
text = text.replace("GC-covered table-less v4 manifest", "GC-covered table-less v5 manifest")
text = text.replace("writes/synchronizes a new Manifest v4 naming the new WAL", "writes/synchronizes a new Manifest v5 naming the new WAL")
old = '''filtering is part of SSTable v2; level metadata and full-set L0-to-L1 publication remain readable from
Manifest v3; the GC proof watermark and table-less durable state are Manifest v4. WAL segment
rotation/reclamation remains part of the implemented crash-state protocol.'''
new = '''filtering is part of SSTable v2; level metadata and full-set L0-to-L1 publication remain readable from
Manifest v3; the GC proof watermark and table-less durable state originated in Manifest v4; persistent
SSTable allocation history across orphan cleanup is Manifest v5. WAL segment rotation/reclamation remains
part of the implemented crash-state protocol.'''
text = replace_once(text, old, new, "explicitly deferred version summary")
p.write_text(text)


# WAL format cross-reference
p = Path("docs/lsm-wal-format.md")
text = p.read_text()
text = replace_once(
    text,
    "Manifest-v4 L0/L1 compaction and tombstone-GC protocol are specified in",
    "Manifest-v5 L0/L1 compaction, tombstone-GC, and SSTable-allocation protocol is specified in",
    "WAL intro cross-reference",
)
text = replace_once(
    text,
    "unreferenced crash orphans. Manifest v2, v3, and v4 identify exactly one authoritative WAL by id and\nfirst sequence.",
    "unreferenced crash orphans. Manifest v2 through v5 identify exactly one authoritative WAL by id and\nfirst sequence. Manifest v5 independently persists the SSTable allocation high watermark; it does not\nchange WAL ids or WAL record bytes.",
    "WAL manifest versions",
)
p.write_text(text)


# Roadmap
p = Path("docs/roadmap.md")
text = p.read_text()
needle = '''- [x] Prove and implement safe tombstone dropping for the current serialized, snapshot-free full-set
  compactor. Manifest v4 records a GC watermark, permits a GC-covered table-less durable version, carries
  the watermark through later flush/WAL rotation, and validates legacy v1–v3 manifests with an implicit
  zero watermark. Tests cover physical elision, reinsertion, fully deleted reopen/refill, corrupt
  watermarks, and old/new crash publication.
'''
addition = needle + '''- [x] Persist the SSTable allocation frontier through table-less GC and orphan cleanup. Manifest v5
  extends the header with a monotonic table-id high watermark while preserving the v4 GC field in place;
  open raises the floor from every canonical SSTable name before cleanup and the next v5 publication
  makes that reservation durable. Tests prove v1–v4 readability, conservative table-less v4 migration,
  checksum-valid invalid-watermark rejection, and crash orphan id 99 being followed by id 100 after the
  orphan name has been removed.
'''
text = replace_once(text, needle, addition, "roadmap v5 milestone")
p.write_text(text)


# Design-space status
p = Path("docs/design-space.md")
text = p.read_text()
old = '''| Binary KV + LSM + standalone | Persistent correctness engine; not yet a performance candidate | `db-storage-lsm` implements the common point/reopen/range contract with checksummed segmented WALs, ordered MemTables, immutable indexed/checksummed SSTables, mirrored CURRENT, crash-safe WAL rotation/reclamation, SSTable v2 embedded Bloom filters, and Manifest-v4 full-set L0/L1 compaction. The serialized snapshot-free full-set proof point elides tombstones, including a GC-covered zero-SSTable state; deterministic fault matrices require complete old-or-new recovery. Generalized levels, snapshot/replication-aware GC, validated amplification counters, and comparable performance evidence remain deferred |'''
new = '''| Binary KV + LSM + standalone | Persistent correctness engine; not yet a performance candidate | `db-storage-lsm` implements the common point/reopen/range contract with checksummed segmented WALs, ordered MemTables, immutable indexed/checksummed SSTables, mirrored CURRENT, crash-safe WAL rotation/reclamation, SSTable v2 embedded Bloom filters, and Manifest-v5 full-set L0/L1 compaction. The serialized snapshot-free full-set proof point elides tombstones, including a GC-covered zero-SSTable state; v5 also preserves the SSTable allocation frontier across canonical-orphan cleanup so ambiguous ids are not reused. Deterministic fault matrices require complete old-or-new recovery. Generalized levels, snapshot/replication-aware GC, validated amplification counters, and comparable performance evidence remain deferred |'''
text = replace_once(text, old, new, "design-space LSM status")
p.write_text(text)
