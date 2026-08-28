from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def replace_once(path: Path, old: str, new: str, label: str) -> None:
    text = path.read_text()
    if text.count(old) != 1:
        raise SystemExit(f"{label}: expected exactly one marker, found {text.count(old)}")
    path.write_text(text.replace(old, new, 1))


lib = ROOT / "crates/db-storage-lsm/src/lib.rs"
replace_once(
    lib,
    "//! the replacement L1 SSTable, Manifest, first CURRENT, and mirror CURRENT boundaries.\n",
    "//! the replacement L1 SSTable, Manifest, first CURRENT, and mirror CURRENT boundaries. A second\n//! deterministic evidence layer compares multi-cycle compaction against the in-memory oracle and exposes\n//! exact integer read/data-write/sorted-table-space amplification counters for hand-computable traces.\n",
    "lib crate docs",
)

readme = ROOT / "README.md"
replace_once(
    readme,
    "| `db-storage-lsm` | Common persistent `KvEngine` with checksummed segmented WALs, ordered MemTables, indexed/checksummed immutable SSTables, validated embedded Bloom filters, Manifest-v5 L0/L1, tombstone-GC, and SSTable-allocation metadata, crash-published full-set compaction, mirrored `CURRENT`, WAL/SSTable/manifest reclamation, and half-open range scans |\n",
    "| `db-storage-lsm` | Common persistent `KvEngine` with checksummed segmented WALs, ordered MemTables, indexed/checksummed immutable SSTables, validated embedded Bloom filters, Manifest-v5 L0/L1, tombstone-GC, and SSTable-allocation metadata, crash-published full-set compaction, mirrored `CURRENT`, WAL/SSTable/manifest reclamation, half-open range scans, deterministic compaction differential tests, and exact integer amplification instrumentation |\n",
    "README role",
)
replace_once(
    readme,
    "Reopen must select either the complete four-L0 input version or the complete GC-published version; no\nmixed version is accepted. The current one-run L1 policy remains correctness evidence rather than a\nproduction leveled strategy, so this is still not a fair B+ tree performance comparison participant.\n",
    "Reopen must select either the complete four-L0 input version or the complete GC-published version; no\nmixed version is accepted. Deterministic two-cycle compaction traces are also checked against the\nin-memory oracle, including overwrite, delete, range, and reopen behavior. Process-local instrumentation\nreports raw integer numerator/denominator pairs for point SSTable consults per GET, SSTable versions\ndecoded per range result, WAL+flush+compaction-output data bytes per acknowledged logical mutation byte,\nand authoritative SSTable bytes per durable live key+value byte. These are deliberately engine-level\nstructural/data-path counters: manifest/CURRENT bytes, filesystem metadata, cache/device traffic, and\nhardware writeback are not silently folded into them. The current one-run L1 policy remains correctness\nevidence rather than a production leveled strategy, so this is still not a fair B+ tree performance\ncomparison participant.\n",
    "README LSM evidence paragraph",
)

fmt = ROOT / "docs/lsm-sstable-manifest-format.md"
replace_once(
    fmt,
    "## Explicitly deferred\n\nGeneralized size-based/multi-run levels, snapshot-aware or replication-aware tombstone GC, block/cache\ndesign, and read/write/space-amplification instrumentation remain separate evidence milestones. Bloom\nfiltering is part of SSTable v2; level metadata and full-set L0-to-L1 publication remain readable from\n",
    "## Amplification instrumentation\n\n`LsmEngine` exposes resettable, process-local raw counters plus exact integer ratio pairs. They are\nmeasurement evidence for this implementation, not device-level telemetry and not benchmark conclusions.\nA logical `reopen` on the same engine handle preserves the counters; constructing a new engine handle\nstarts a new measurement window. Counter saturation cannot change database behavior.\n\nThe implemented ratios are intentionally explicit:\n\n- **point read:** SSTables consulted by successful explicit `GET` calls / explicit `GET` calls. MemTable\n  hits therefore contribute zero sorted-table consults, while an absent key may consult every current run.\n  Internal lookups used to return a previous value from `PUT`/`DELETE` are not counted as user reads.\n- **range read:** physical SSTable records decoded inside successful range scans / logical live records\n  returned. Multiple physical versions of one logical key therefore remain visible in the numerator.\n- **data write:** complete WAL mutation-record bytes + flush SSTable file bytes + compaction-output SSTable\n  file bytes / acknowledged logical mutation bytes (`key + PUT value`, or key only for DELETE). Manifest,\n  `CURRENT`, filesystem metadata, cache traffic, and device writeback are outside this deliberately narrow\n  numerator. `compaction_input_sstable_bytes` is exposed separately as read-side compaction work.\n- **sorted-table space:** bytes in SSTables referenced by the authoritative manifest / durable live\n  key+value bytes represented by those SSTables. Unflushed WAL/MemTable state is excluded from both sides\n  of this sorted-table ratio. A zero denominator is preserved as `0` rather than converted to NaN/infinity.\n\nA hand-computable regression creates four two-entry L0 flushes, verifies that first-compaction input bytes\nequal the sum of those flush outputs, and verifies that the surviving L1 file size equals compaction output\nbytes. It then layers a two-entry L0 over an eight-entry L1: three point reads require exactly 5 SSTable\nconsults (`5/3`), while a full range decodes 10 physical versions and returns 9 logical keys (`10/9`).\nThe same suite checks physical SSTable bytes from directory metadata and exact WAL record framing. A\nseparate deterministic state machine drives two full-set compaction cycles with overwrites and deletes\nagainst `db-storage-memory`, including range comparisons and reopen after each compacted version.\n\n## Explicitly deferred\n\nGeneralized size-based/multi-run levels, snapshot-aware or replication-aware tombstone GC, block/cache\ndesign, common cross-engine amplification counters, and device-level I/O attribution remain separate\nevidence milestones. Bloom\nfiltering is part of SSTable v2; level metadata and full-set L0-to-L1 publication remain readable from\n",
    "format instrumentation section",
)

roadmap = ROOT / "docs/roadmap.md"
replace_once(
    roadmap,
    "- [ ] Add deterministic compaction differential tests and read/write/space-amplification instrumentation\n  validation.\n",
    "- [x] Add deterministic compaction differential tests and read/write/space-amplification instrumentation\n  validation. Two full-set compaction cycles are checked against the in-memory oracle across overwrites,\n  deletes, ranges, and reopen. Resettable process-local counters expose exact integer point-read, range-read,\n  data-write, and sorted-table-space ratios; hand-computable tests prove 5/3 point consults, 10/9 range\n  versions/results, WAL framing, first-compaction input=flush-output bytes, and authoritative SSTable sizes.\n",
    "roadmap phase3 milestone",
)
replace_once(
    roadmap,
    "- [ ] Validate counters for read, write, and space amplification on hand-computable traces.\n",
    "- [ ] Generalize validated read/write/space amplification counters into a common cross-engine experiment\n  contract. LSM-local hand-computable counters are now proven; B+ tree parity and shared reporting remain.\n",
    "roadmap phase4 counter milestone",
)

design = ROOT / "docs/design-space.md"
replace_once(
    design,
    "| Binary KV + LSM + standalone | Persistent correctness engine; not yet a performance candidate | `db-storage-lsm` implements the common point/reopen/range contract with checksummed segmented WALs, ordered MemTables, immutable indexed/checksummed SSTables, mirrored CURRENT, crash-safe WAL rotation/reclamation, SSTable v2 embedded Bloom filters, and Manifest-v5 full-set L0/L1 compaction. The serialized snapshot-free full-set proof point elides tombstones, including a GC-covered zero-SSTable state; v5 also preserves the SSTable allocation frontier across canonical-orphan cleanup so ambiguous ids are not reused. Deterministic fault matrices require complete old-or-new recovery. Generalized levels, snapshot/replication-aware GC, validated amplification counters, and comparable performance evidence remain deferred |\n",
    "| Binary KV + LSM + standalone | Persistent correctness engine with local amplification evidence; not yet a performance candidate | `db-storage-lsm` implements the common point/reopen/range contract with checksummed segmented WALs, ordered MemTables, immutable indexed/checksummed SSTables, mirrored CURRENT, crash-safe WAL rotation/reclamation, SSTable v2 embedded Bloom filters, and Manifest-v5 full-set L0/L1 compaction. The serialized snapshot-free full-set proof point elides tombstones, including a GC-covered zero-SSTable state; v5 also preserves the SSTable allocation frontier across canonical-orphan cleanup so ambiguous ids are not reused. Deterministic fault matrices require complete old-or-new recovery, and deterministic multi-cycle compaction is differentially checked against the memory oracle. Exact integer LSM-local point/range/data-write/sorted-table-space counters are validated on hand-computable traces. Generalized levels, snapshot/replication-aware GC, common cross-engine metrics, controlled-host measurements, and comparable performance evidence remain deferred |\n",
    "design-space LSM status",
)
