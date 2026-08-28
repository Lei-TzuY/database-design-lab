from pathlib import Path


def replace_once(path: str, old: str, new: str, label: str) -> None:
    p = Path(path)
    text = p.read_text()
    if new in text:
        return
    if old not in text:
        raise SystemExit(f"missing {label} in {path}: {old[:220]!r}")
    p.write_text(text.replace(old, new, 1))

# Crate-level implementation summary.
replace_once(
    "crates/db-storage-lsm/src/lib.rs",
    "//! the replacement L1 SSTable, Manifest, first CURRENT, and mirror CURRENT boundaries.\n",
    "//! the replacement L1 SSTable, Manifest, first CURRENT, and mirror CURRENT boundaries. Manifest v4\n//! persists the observed SSTable-id high watermark, permits a durable empty version set, and lets full-set\n//! compaction elide tombstones only after every older table version is part of the same merge.\n",
    "crate v4 summary",
)

# README status and design narrative.
replace_once(
    "README.md",
    "| `db-storage-lsm` | Common persistent `KvEngine` with checksummed segmented WALs, ordered MemTables, indexed/checksummed immutable SSTables, validated embedded Bloom filters, Manifest-v3 L0/L1 metadata, crash-published full-set compaction, mirrored `CURRENT`, WAL/SSTable/manifest reclamation, tombstones, and half-open range scans |",
    "| `db-storage-lsm` | Common persistent `KvEngine` with checksummed segmented WALs, ordered MemTables, indexed/checksummed immutable SSTables, validated embedded Bloom filters, Manifest-v4 L0/L1 metadata and allocation history, crash-published full-set compaction, safe tombstone elision with durable-empty checkpoints, mirrored `CURRENT`, WAL/SSTable/manifest reclamation, and half-open range scans |",
    "README LSM role",
)
replace_once(
    "README.md",
    "full-set merge of all authoritative SSTables into one L1 run. The compacted SSTable and Manifest v3 are\nsynchronized, the same manifest is published through both CURRENT mirrors, and only then are obsolete\nSSTables/manifests eligible for best-effort deletion. Deterministic fault injection now covers the\nreplacement L1 SSTable, Manifest v3, first CURRENT publication, and mirror CURRENT publication under\nbefore-write, torn-output, and post-sync reported failures. Reopen is required to select either the\ncomplete four-L0 input version or the complete one-L1 compacted version; no mixed version is accepted.\nTombstones are deliberately retained, and the current one-run L1 policy is correctness evidence rather\nthan a production leveled strategy, so this is still not a fair B+ tree performance comparison participant.",
    "full-set merge of all authoritative SSTables. Manifest v4 keeps v3's explicit L0/L1 descriptors and\nadds a persistent SSTable-id high watermark. Full-set compaction keeps the newest sequence per key and\nmay then physically elide tombstones because no older SSTable version remains outside the merge. A\nnonempty result becomes one L1 SSTable; an all-deleted result becomes a durable-empty checkpoint with\n`durable_sequence > 0` and zero SSTables. The replacement state is synchronized, the same manifest is\npublished through both CURRENT mirrors, and only then are obsolete SSTables/manifests eligible for\nbest-effort deletion. The persisted id watermark also reserves canonical crash-orphan ids even when an\nempty compaction later deletes those files; a regression proves orphan id 99 is followed by table 100\nafter cleanup/reopen. Deterministic fault injection covers nonempty and empty compaction publication,\nrequiring reopen to select one complete old or new version and never a mixed state. The current one-run\nL1 policy remains correctness evidence rather than a production leveled strategy, so this is still not\na fair B+ tree performance comparison participant.",
    "README v4 narrative",
)
replace_once(
    "README.md",
    "multi-process writers, safe LSM tombstone dropping, generalized multi-run/multi-level compaction,\nreplication, SQL, MVCC, Raft, graph, time-series, and columnar execution are not implemented.",
    "multi-process writers, generalized multi-run/multi-level compaction, compaction/amplification\ninstrumentation, replication, SQL, MVCC, Raft, graph, time-series, and columnar execution are not implemented.",
    "README deferred list",
)

# WAL document delegates v4 version-set semantics; WAL bytes remain v1.
replace_once(
    "docs/lsm-wal-format.md",
    "Manifest-v3 L0/L1 compaction protocol are specified in `docs/lsm-sstable-manifest-format.md`. The WAL\nrecord bytes do not change for compaction.",
    "Manifest-v4 L0/L1 compaction, durable-empty checkpoint, and tombstone-elision protocol are specified\nin `docs/lsm-sstable-manifest-format.md`. The WAL record bytes do not change for compaction or empty\ncheckpoints.",
    "WAL v4 delegation",
)
replace_once(
    "docs/lsm-wal-format.md",
    "unreferenced crash orphans. Manifest v2 and v3 identify exactly one authoritative WAL by id and first sequence.",
    "unreferenced crash orphans. Manifest v2, v3, and v4 identify exactly one authoritative WAL by id and first sequence.",
    "WAL manifest versions",
)

# Main persistent-format document.
replace_once(
    "docs/lsm-sstable-manifest-format.md",
    "# LSM SSTable, manifest, WAL, and L0/L1 compaction v3",
    "# LSM SSTable, manifest, WAL, and L0/L1 compaction v4",
    "format title",
)
replace_once(
    "docs/lsm-sstable-manifest-format.md",
    "SSTable v2 embeds a validated Bloom filter. Manifest v3 additionally records each table's level and\nimplements a correctness-first overlapping-L0 / single-run-L1 compaction policy. Safe tombstone dropping\nremains outside this version.",
    "SSTable v2 embeds a validated Bloom filter. Manifest v3 introduced explicit table levels and the\ncorrectness-first overlapping-L0 / single-run-L1 policy. Manifest v4 keeps the descriptor encoding,\npersists SSTable-id allocation history, permits durable-empty checkpoints, and allows safe tombstone\nelision only during full-set compaction.",
    "format intro",
)
replace_once(
    "docs/lsm-sstable-manifest-format.md",
    "WAL id, WAL first sequence, reserved zeros, and header CRC. Manifest v3 keeps that 80-byte header and\nchanges only the SSTable descriptor body.\n\nManifest v1/v2 descriptors have a 40-byte prefix and are interpreted as level 0 for compatibility. New\nManifest v3 descriptors use a 48-byte prefix: table id, file bytes, entry count, durable sequence, a u32\nlevel, a zero u32 reserved field, smallest/largest-key lengths, key bounds, and descriptor CRC. The\nimplemented policy accepts only levels 0 and 1 and at most one L1 descriptor. Descriptors remain ordered\nby strictly increasing table id and durable sequence. The manifest-level durable sequence must equal the\nnewest descriptor's watermark, or zero when the table set is empty. No manifest is accepted merely because\nthe WAL could reconstruct its data: a manifest selected by CURRENT is authoritative persistent state, so\nchecksum, level, or descriptor corruption fails closed.",
    "WAL id, WAL first sequence, reserved zeros, and header CRC. Manifest v3 keeps that 80-byte header and\nchanges the SSTable descriptor body. Manifest v4 keeps the v3 descriptor bytes and assigns header bytes\n64..72 to a persistent `table_id_high_watermark`; bytes 72..76 remain reserved zero. The header CRC still\ncovers bytes 0..75.\n\nManifest v1/v2 descriptors have a 40-byte prefix and are interpreted as level 0 for compatibility.\nManifest v3/v4 descriptors use a 48-byte prefix: table id, file bytes, entry count, durable sequence, a\nu32 level, a zero u32 reserved field, smallest/largest-key lengths, key bounds, and descriptor CRC. v1-v3\nfiles remain readable and derive their in-memory table-id watermark from the largest active descriptor.\nNew v4 files persist the greater allocation history explicitly, including canonical crash-orphan ids that\nmay later be deleted. The watermark must never be below an active descriptor id. The implemented policy\naccepts only levels 0 and 1 and at most one L1 descriptor. For nonempty version sets, the manifest durable\nsequence still equals the newest descriptor watermark. Manifest v4 additionally permits `tables = []`\nwith `durable_sequence > 0`: this is a durable-empty checkpoint proving that full-set compaction has\nconsumed history through that sequence. Such a checkpoint requires a nonzero table-id watermark. Semantic\ncorruption tests recompute valid header/file CRCs after lowering the watermark and still require open to\nfail closed, proving these are format invariants rather than checksum accidents.",
    "manifest v4 section",
)
replace_once(
    "docs/lsm-sstable-manifest-format.md",
    "synchronously compacts **all** authoritative SSTables (the existing L1 run, if present, plus every L0)\ninto one new level-1 SSTable. This deliberately simple policy means L1 is a single sorted non-overlapping\nrun rather than a production size-tiered or multi-run leveled design. A later flush again creates L0 and\nmay override L1 by sequence; four later L0 tables trigger another full-set rewrite.\n\nCompaction overlays every input key by highest sequence, preserving explicit tombstones. Tombstones are\nnot elided in v3 even though full-set compaction creates a useful future proof point: safe dropping is a\nseparate milestone so deletion history is never discarded by an unstated assumption.",
    "synchronously compacts **all** authoritative SSTables (the existing L1 run, if present, plus every L0).\nThis deliberately simple policy means L1 is at most one sorted non-overlapping run rather than a production\nsize-tiered or multi-run leveled design. A later flush again creates L0 and may override L1 by sequence;\nfour later L0 tables trigger another full-set rewrite.\n\nCompaction first overlays every input key by highest sequence, then drops entries whose newest state is a\ntombstone. This is safe specifically because the input set is exhaustive: after the compacted version is\npublished, there is no older authoritative SSTable outside the merge that a removed tombstone would need\nto suppress. WAL replay applies only records with sequence greater than the manifest durable watermark.\nIf live values remain, they are written to one replacement L1. If every newest entry is a tombstone, no\nreplacement SSTable is created; Manifest v4 records the same durable watermark and an empty table set.",
    "compaction tombstone semantics",
)
replace_once(
    "docs/lsm-sstable-manifest-format.md",
    "1. Keep the old manifest/SSTables authoritative while reading all compaction inputs.\n2. Create the replacement L1 SSTable at a fresh canonical id and `sync_all` it.\n3. Create and `sync_all` a new Manifest v3 that references only that L1 output and the unchanged active WAL.\n4. Publish the new manifest through the next CURRENT generation and `sync_data`.\n5. Publish the **same manifest id** through the other CURRENT mirror at generation + 1 and `sync_data`.\n6. Only after both mirrors are self-contained on the compacted version may the live engine drop old table\n   handles and best-effort remove obsolete canonical SSTables and manifest snapshots.",
    "1. Keep the old manifest/SSTables authoritative while reading all compaction inputs.\n2. If live values remain, create the replacement L1 SSTable at a fresh canonical id and `sync_all` it;\n   an empty result deliberately skips this step.\n3. Create and `sync_all` a new Manifest v4 that references the optional L1 output, unchanged active WAL,\n   durable sequence, and observed SSTable-id high watermark.\n4. Publish the new manifest through the next CURRENT generation and `sync_data`.\n5. Publish the **same manifest id** through the other CURRENT mirror at generation + 1 and `sync_data`.\n6. Only after both mirrors are self-contained on the compacted version may the live engine drop old table\n   handles and best-effort remove obsolete canonical SSTables and manifest snapshots.",
    "compaction v4 ordering",
)
replace_once(
    "docs/lsm-sstable-manifest-format.md",
    "tests corrupt the newer CURRENT mirror after obsolete-file cleanup and require the older mirror to reopen\nthe single L1 run successfully. They also verify tombstone retention and a newer L0 value overriding an\nolder L1 tombstone across reopen.",
    "tests corrupt the newer CURRENT mirror after obsolete-file cleanup and require the older mirror to reopen\nthe compacted version successfully. They verify physical tombstone elision, a newer L0 value overriding\ncompacted state across reopen, and a 64-tombstone workload compacting to `durable_sequence = 64` with zero\nSSTables. Reopen/verify preserve that empty checkpoint, and the next flushed table continues above the\npersisted id high watermark rather than reusing old ids. A crash-retry regression adds canonical orphan\nSSTable id 99, publishes an empty checkpoint that cleans it up, then requires the next table to be id 100.",
    "compaction v4 recovery evidence",
)
replace_once(
    "docs/lsm-sstable-manifest-format.md",
    "The compaction fault matrix records the four durable publication classes in order: replacement L1\nSSTable, Manifest v3, first CURRENT slot, then mirror CURRENT slot. Each class is exercised under a\nbefore-write error, a synchronized torn-output aftermath, and an after-sync reported error. Torn\nSSTable/Manifest cases truncate the new immutable file to half its committed extent; torn CURRENT cases\noverwrite half of the selected 4 KiB slot and synchronize the damaged bytes. The triggering live handle\nis poisoned and must be reopened. Before the first CURRENT becomes fully durable, reopen must select the\ncomplete four-L0 input version; an after-sync error from the first CURRENT and every mirror-stage error\nselect the complete one-L1 version. Every case rechecks all logical keys plus `verify`, so a structurally\nmixed publication cannot pass merely because point reads happen to agree.",
    "The nonempty compaction fault matrix records four durable publication classes in order: replacement L1\nSSTable, Manifest v4, first CURRENT slot, then mirror CURRENT slot. Each class is exercised under a\nbefore-write error, a synchronized torn-output aftermath, and an after-sync reported error. Torn\nSSTable/Manifest cases truncate the new immutable file to half its committed extent; torn CURRENT cases\noverwrite half of the selected 4 KiB slot and synchronize the damaged bytes. The triggering live handle\nis poisoned and must be reopened. Before the first CURRENT becomes fully durable, reopen selects the\ncomplete four-L0 input version; an after-sync error from the first CURRENT and every mirror-stage error\nselects the complete one-L1 version. Empty-output compaction has no L1 write, so its stable trace is\nManifest -> first CURRENT -> mirror CURRENT; all three classes receive the same three fault modes in a\nseparate nine-case matrix, and reopen must select either four L0 tombstone tables or the durable-empty\ncheckpoint. Every case rechecks logical results plus `verify`, so a structurally mixed publication cannot\npass merely because deleted point reads happen to agree.",
    "v4 fault matrix",
)
replace_once(
    "docs/lsm-sstable-manifest-format.md",
    "`durable_sequence + 1`, writes/synchronizes a new Manifest v3 naming the new WAL and unchanged SSTable\nset, and publishes that manifest through the next CURRENT generation.",
    "`durable_sequence + 1`, writes/synchronizes a new Manifest v4 naming the new WAL and unchanged optional\nSSTable set, and publishes that manifest through the next CURRENT generation.",
    "WAL rotation v4",
)
replace_once(
    "docs/lsm-sstable-manifest-format.md",
    "bounds/limit. The current L0/L1 compactor removes superseded physical table versions only after\ndouble-mirror publication, but deliberately retains logical tombstones.\n\n## Explicitly deferred\n\nSafe tombstone elision, generalized size-based/multi-run levels, block/cache design, and\nread/write/space-amplification instrumentation remain separate evidence milestones. Bloom\nfiltering is part of SSTable v2; level metadata and full-set L0-to-L1 publication are part of Manifest v3;\nWAL segment rotation/reclamation remains part of the implemented crash-state protocol.",
    "bounds/limit. The current full-set L0/L1 compactor removes superseded physical table versions and safe\nlogical tombstones only after crash-safe version publication; an all-deleted state is represented by a\nManifest v4 durable-empty checkpoint rather than a dummy SSTable.\n\n## Explicitly deferred\n\nGeneralized size-based/multi-run levels, block/cache design, deterministic compaction differential traces,\nand read/write/space-amplification instrumentation remain separate evidence milestones. Bloom filtering\nis part of SSTable v2; explicit levels began in Manifest v3; persistent allocation history, durable-empty\ncheckpoints, and safe full-set tombstone elision are part of Manifest v4; WAL segment rotation/reclamation\nremains part of the implemented crash-state protocol.",
    "v4 deferred section",
)

# Roadmap: safe tombstone elision is now executable evidence.
replace_once(
    "docs/roadmap.md",
    "- [x] Add an explicit L0/L1 overlap policy and crash-published compaction. Manifest v3 records levels;\n  flushes enter overlapping L0 and four L0 tables trigger a full-set rewrite into one L1 run. The output\n  SSTable and manifest are synchronized and published through both CURRENT mirrors before obsolete\n  SSTables/manifests are eligible for deletion. Tests cover reopen, mirror fallback after cleanup, retained\n  tombstones, and newer L0 state overriding L1.\n- [ ] Prove and implement safe tombstone dropping; compaction v3 deliberately retains deletion markers.",
    "- [x] Add an explicit L0/L1 overlap policy and crash-published compaction. Manifest v3 records levels;\n  flushes enter overlapping L0 and four L0 tables trigger a full-set rewrite into at most one L1 run. The\n  replacement state and manifest are synchronized and published through both CURRENT mirrors before\n  obsolete SSTables/manifests are eligible for deletion. Tests cover reopen, mirror fallback after cleanup,\n  and newer L0 state overriding compacted L1 state.\n- [x] Prove and implement safe full-set tombstone elision with Manifest v4 durable-empty checkpoints. v4\n  persists an SSTable-id high watermark in header bytes 64..72, keeps v1-v3 readable, and permits a\n  positive durable sequence with zero tables after all newest entries are deletions. Tests prove physical\n  tombstone removal, zero-SSTable reopen/verify, post-empty allocation continuing at table 5, crash-orphan\n  id 99 forcing later table 100 even after cleanup, v3-to-v4 upgrade, and fail-closed semantic corruption.",
    "roadmap v4 milestone",
)
