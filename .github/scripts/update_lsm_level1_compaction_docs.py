from pathlib import Path


def replace_once(path: Path, old: str, new: str, label: str) -> None:
    text = path.read_text()
    if old not in text:
        raise SystemExit(f"missing {label} in {path}: {old[:160]!r}")
    path.write_text(text.replace(old, new, 1))

# README status and scope.
p = Path("README.md")
replace_once(
    p,
    "| `db-storage-lsm` | Common persistent `KvEngine` with checksummed segmented WALs, ordered MemTables, indexed/checksummed immutable SSTables, validated embedded Bloom filters, immutable manifest snapshots, mirrored `CURRENT` publication, crash-safe WAL rotation/reclamation, tombstones, and half-open range scans; no levels or compaction yet |",
    "| `db-storage-lsm` | Common persistent `KvEngine` with checksummed segmented WALs, ordered MemTables, indexed/checksummed immutable SSTables, validated embedded Bloom filters, Manifest-v3 L0/L1 metadata, crash-published full-set compaction, mirrored `CURRENT`, WAL/SSTable/manifest reclamation, tombstones, and half-open range scans |",
    "README LSM table row",
)
replace_once(
    p,
    "silently introduce a false negative. Levels and compaction remain absent, so this is still not a fair\nB+ tree performance comparison participant.",
    "silently introduce a false negative. Flushes enter overlapping L0; four L0 tables trigger a synchronous\nfull-set merge of all authoritative SSTables into one L1 run. The compacted SSTable and Manifest v3 are\nsynchronized, the same manifest is published through both CURRENT mirrors, and only then are obsolete\nSSTables/manifests eligible for best-effort deletion. Tombstones are deliberately retained, and the\ncurrent one-run L1 policy is correctness evidence rather than a production leveled strategy, so this is\nstill not a fair B+ tree performance comparison participant.",
    "README LSM narrative",
)
replace_once(
    p,
    "multi-process writers, LSM levels/compaction, replication, SQL, MVCC, Raft, graph, time-series,\nand columnar execution are not implemented.",
    "multi-process writers, safe LSM tombstone dropping, generalized multi-run/multi-level compaction,\nreplication, SQL, MVCC, Raft, graph, time-series, and columnar execution are not implemented.",
    "README non-goals",
)

# WAL doc only delegates the new version-set/compaction protocol to the SSTable/manifest spec.
p = Path("docs/lsm-wal-format.md")
replace_once(
    p,
    "but WALs are now canonical numbered segments selected by Manifest v2. Indexed SSTables, immutable\nmanifest snapshots, mirrored `CURRENT`, flush publication, and the cross-file rotation/reclamation\nprotocol and SSTable v2 Bloom filters are specified with `docs/lsm-sstable-manifest-format.md`. Levels\nand compaction remain deferred. Consequently the engine is executable correctness evidence, but not yet a candidate for\nB+ tree versus LSM performance claims.",
    "but WALs are canonical numbered segments selected by the manifest version set. Indexed SSTables,\nimmutable manifests, mirrored `CURRENT`, flush/WAL publication, SSTable v2 Bloom filters, and the\nManifest-v3 L0/L1 compaction protocol are specified in `docs/lsm-sstable-manifest-format.md`. The WAL\nrecord bytes do not change for compaction. The engine remains correctness evidence, not yet a candidate\nfor B+ tree versus LSM performance claims.",
    "WAL intro",
)
replace_once(
    p,
    "Manifest v2 identifies exactly one authoritative WAL by id and first sequence.",
    "Manifest v2 and v3 identify exactly one authoritative WAL by id and first sequence.",
    "WAL manifest authority wording",
)

# Main sorted-table/version-set specification.
p = Path("docs/lsm-sstable-manifest-format.md")
replace_once(p, "# LSM SSTable, manifest, and WAL publication v2", "# LSM SSTable, manifest, WAL, and L0/L1 compaction v3", "format title")
replace_once(
    p,
    "`CURRENT`; when that durable watermark reaches the active WAL tail, Manifest v2 can atomically switch the\nversion set to a new empty WAL and reclaim older segments only after both CURRENT mirrors move. SSTable\nv2 now embeds a validated Bloom filter; levels, compaction, and tombstone dropping remain outside this\nversion.",
    "`CURRENT`; when that durable watermark reaches the active WAL tail, the manifest can atomically switch\nthe version set to a new empty WAL and reclaim older segments only after both CURRENT mirrors move.\nSSTable v2 embeds a validated Bloom filter. Manifest v3 additionally records each table's level and\nimplements a correctness-first overlapping-L0 / single-run-L1 compaction policy. Safe tombstone dropping\nremains outside this version.",
    "format intro",
)
replace_once(
    p,
    "A `MANIFEST-%016d` is a complete version-set snapshot, not an append log. Manifest v1 uses its original\n64-byte header and remains readable with implicit WAL id 1 / first sequence 1. Newly written Manifest v2\nuses an 80-byte header that stores manifest id, durable sequence, table count, descriptor-body length,\na nonzero authoritative WAL id, and that WAL's nonzero first sequence, followed by reserved zeros and a\nheader CRC. Every SSTable descriptor is individually checksummed, and the entire manifest has a trailing\nCRC.\n\nDescriptors are ordered by strictly increasing table id and durable sequence. The manifest-level durable\nsequence must equal the newest descriptor's watermark, or zero when the table set is empty. No manifest\nis accepted merely because the WAL could reconstruct its data: a manifest selected by CURRENT is part\nof authoritative persistent state, so checksum or descriptor corruption fails closed.",
    "A `MANIFEST-%016d` is a complete version-set snapshot, not an append log. Manifest v1 uses its original\n64-byte header and remains readable with implicit WAL id 1 / first sequence 1. Manifest v2 introduced the\n80-byte header containing manifest id, durable sequence, table count, descriptor-body length, authoritative\nWAL id, WAL first sequence, reserved zeros, and header CRC. Manifest v3 keeps that 80-byte header and\nchanges only the SSTable descriptor body.\n\nManifest v1/v2 descriptors have a 40-byte prefix and are interpreted as level 0 for compatibility. New\nManifest v3 descriptors use a 48-byte prefix: table id, file bytes, entry count, durable sequence, a u32\nlevel, a zero u32 reserved field, smallest/largest-key lengths, key bounds, and descriptor CRC. The\nimplemented policy accepts only levels 0 and 1 and at most one L1 descriptor. Descriptors remain ordered\nby strictly increasing table id and durable sequence. The manifest-level durable sequence must equal the\nnewest descriptor's watermark, or zero when the table set is empty. No manifest is accepted merely because\nthe WAL could reconstruct its data: a manifest selected by CURRENT is authoritative persistent state, so\nchecksum, level, or descriptor corruption fails closed.",
    "manifest v3 section",
)
insert_marker = "## Mirrored CURRENT publication\n"
text = p.read_text()
if insert_marker not in text:
    raise SystemExit("missing CURRENT section marker")
compaction = '''## Level policy and full-set compaction\n\nEvery normal MemTable flush is published as level 0. L0 tables may overlap arbitrarily; sequence numbers,\nnot key-range placement, decide which version wins. When four L0 descriptors are authoritative, the engine\nsynchronously compacts **all** authoritative SSTables (the existing L1 run, if present, plus every L0)\ninto one new level-1 SSTable. This deliberately simple policy means L1 is a single sorted non-overlapping\nrun rather than a production size-tiered or multi-run leveled design. A later flush again creates L0 and\nmay override L1 by sequence; four later L0 tables trigger another full-set rewrite.\n\nCompaction overlays every input key by highest sequence, preserving explicit tombstones. Tombstones are\nnot elided in v3 even though full-set compaction creates a useful future proof point: safe dropping is a\nseparate milestone so deletion history is never discarded by an unstated assumption.\n\nThe crash-publication order is:\n\n1. Keep the old manifest/SSTables authoritative while reading all compaction inputs.\n2. Create the replacement L1 SSTable at a fresh canonical id and `sync_all` it.\n3. Create and `sync_all` a new Manifest v3 that references only that L1 output and the unchanged active WAL.\n4. Publish the new manifest through the next CURRENT generation and `sync_data`.\n5. Publish the **same manifest id** through the other CURRENT mirror at generation + 1 and `sync_data`.\n6. Only after both mirrors are self-contained on the compacted version may the live engine drop old table\n   handles and best-effort remove obsolete canonical SSTables and manifest snapshots.\n\nTherefore interruption before the first CURRENT publication leaves the complete input version authoritative;\nbetween the two CURRENT writes both complete versions still have their physical inputs; after the second\nwrite either valid mirror selects the same compacted manifest, so old files are redundant. Deterministic\ntests corrupt the newer CURRENT mirror after obsolete-file cleanup and require the older mirror to reopen\nthe single L1 run successfully. They also verify tombstone retention and a newer L0 value overriding an\nolder L1 tombstone across reopen.\n\n'''
p.write_text(text.replace(insert_marker, compaction + insert_marker, 1))
replace_once(
    p,
    "`durable_sequence + 1`, writes/synchronizes a new Manifest v2 naming the new WAL and unchanged SSTable\nset, and publishes that manifest through the next CURRENT generation.",
    "`durable_sequence + 1`, writes/synchronizes a new Manifest v3 naming the new WAL and unchanged SSTable\nset, and publishes that manifest through the next CURRENT generation.",
    "rotation manifest version",
)
replace_once(
    p,
    "bounds/limit. Compaction is still required\nbefore old SSTable versions or tombstones can be discarded safely.\n\n## Explicitly deferred\n\nLeveled placement, overlap rules, compaction selection, crash-safe compaction publication, obsolete\nSSTable/manifest deletion, tombstone elision, block/cache design, and amplification instrumentation remain\nseparate evidence milestones. Bloom filtering is now part of SSTable v2. WAL segment rotation/reclamation is now part of the\nimplemented crash-state protocol; parent-directory durability remains subject to the repository-wide\nportable-fsync caveat.",
    "bounds/limit. The current L0/L1 compactor removes superseded physical table versions only after\ndouble-mirror publication, but deliberately retains logical tombstones.\n\n## Explicitly deferred\n\nSafe tombstone elision, generalized size-based/multi-run levels, block/cache design, compaction fault\ninjection, and read/write/space-amplification instrumentation remain separate evidence milestones. Bloom\nfiltering is part of SSTable v2; level metadata and full-set L0-to-L1 publication are part of Manifest v3;\nWAL segment rotation/reclamation remains part of the implemented crash-state protocol. Parent-directory\ndurability remains subject to the repository-wide portable-fsync caveat.",
    "deferred section",
)

# Roadmap: split the formerly bundled levels+compaction+tombstone task.
p = Path("docs/roadmap.md")
replace_once(
    p,
    "- [ ] Add levels, overlap policy, compaction, obsolete-file deletion, and safe tombstone dropping.\n- [ ] Add compaction fault injection, deterministic differential tests, and instrumentation validation.",
    "- [x] Add an explicit L0/L1 overlap policy and crash-published compaction. Manifest v3 records levels;\n  flushes enter overlapping L0 and four L0 tables trigger a full-set rewrite into one L1 run. The output\n  SSTable and manifest are synchronized and published through both CURRENT mirrors before obsolete\n  SSTables/manifests are eligible for deletion. Tests cover reopen, mirror fallback after cleanup, retained\n  tombstones, and newer L0 state overriding L1.\n- [ ] Prove and implement safe tombstone dropping; compaction v3 deliberately retains deletion markers.\n- [ ] Add compaction fault injection, deterministic differential tests, and instrumentation validation.",
    "roadmap compaction split",
)
