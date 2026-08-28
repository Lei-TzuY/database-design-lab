from pathlib import Path


def replace_once(path: str, old: str, new: str, label: str) -> None:
    p = Path(path)
    text = p.read_text()
    if new in text:
        return
    if old not in text:
        raise SystemExit(f"missing {label} in {path}: {old[:180]!r}")
    p.write_text(text.replace(old, new, 1))

replace_once(
    "crates/db-storage-lsm/src/lib.rs",
    "//! mirrored CURRENT before obsolete sorted-table/manifest files are reclaimed.\n",
    "//! mirrored CURRENT before obsolete sorted-table/manifest files are reclaimed. Deterministic\n//! compaction fault tests exercise pre-write, torn durable output, and post-sync reported errors at\n//! the replacement L1 SSTable, Manifest, first CURRENT, and mirror CURRENT boundaries.\n",
    "crate-level compaction fault summary",
)

replace_once(
    "README.md",
    "SSTables/manifests eligible for best-effort deletion. Tombstones are deliberately retained, and the\ncurrent one-run L1 policy is correctness evidence rather than a production leveled strategy, so this is\nstill not a fair B+ tree performance comparison participant.",
    "SSTables/manifests eligible for best-effort deletion. Deterministic fault injection now covers the\nreplacement L1 SSTable, Manifest v3, first CURRENT publication, and mirror CURRENT publication under\nbefore-write, torn-output, and post-sync reported failures. Reopen is required to select either the\ncomplete four-L0 input version or the complete one-L1 compacted version; no mixed version is accepted.\nTombstones are deliberately retained, and the current one-run L1 policy is correctness evidence rather\nthan a production leveled strategy, so this is still not a fair B+ tree performance comparison participant.",
    "README compaction fault evidence",
)

replace_once(
    "docs/lsm-sstable-manifest-format.md",
    "tests corrupt the newer CURRENT mirror after obsolete-file cleanup and require the older mirror to reopen\nthe single L1 run successfully. They also verify tombstone retention and a newer L0 value overriding an\nolder L1 tombstone across reopen.",
    "tests corrupt the newer CURRENT mirror after obsolete-file cleanup and require the older mirror to reopen\nthe single L1 run successfully. They also verify tombstone retention and a newer L0 value overriding an\nolder L1 tombstone across reopen.\n\nThe compaction fault matrix records the four durable publication classes in order: replacement L1\nSSTable, Manifest v3, first CURRENT slot, then mirror CURRENT slot. Each class is exercised under a\nbefore-write error, a synchronized torn-output aftermath, and an after-sync reported error. Torn\nSSTable/Manifest cases truncate the new immutable file to half its committed extent; torn CURRENT cases\noverwrite half of the selected 4 KiB slot and synchronize the damaged bytes. The triggering live handle\nis poisoned and must be reopened. Before the first CURRENT becomes fully durable, reopen must select the\ncomplete four-L0 input version; an after-sync error from the first CURRENT and every mirror-stage error\nselect the complete one-L1 version. Every case rechecks all logical keys plus `verify`, so a structurally\nmixed publication cannot pass merely because point reads happen to agree.",
    "format fault matrix evidence",
)

replace_once(
    "docs/lsm-sstable-manifest-format.md",
    "Safe tombstone elision, generalized size-based/multi-run levels, block/cache design, compaction fault\ninjection, and read/write/space-amplification instrumentation remain separate evidence milestones.",
    "Safe tombstone elision, generalized size-based/multi-run levels, block/cache design, and\nread/write/space-amplification instrumentation remain separate evidence milestones.",
    "format deferred list",
)

replace_once(
    "docs/roadmap.md",
    "- [ ] Prove and implement safe tombstone dropping; compaction v3 deliberately retains deletion markers.\n- [ ] Add compaction fault injection, deterministic differential tests, and instrumentation validation.",
    "- [ ] Prove and implement safe tombstone dropping; compaction v3 deliberately retains deletion markers.\n- [x] Add deterministic compaction durable-write fault injection. The harness records replacement-L1,\n  Manifest, first-CURRENT, and mirror-CURRENT publication classes and injects before-write, synchronized\n  torn-output, and post-sync reported failures at each class. Every case poisons the live handle, reopens,\n  verifies all logical keys, and requires exactly the complete four-L0 input version or complete one-L1\n  compacted version; torn immutable files remain unreferenced and torn CURRENT slots fail by checksum.\n- [ ] Add deterministic compaction differential tests and read/write/space-amplification instrumentation\n  validation.",
    "roadmap compaction evidence split",
)
