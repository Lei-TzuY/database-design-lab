from pathlib import Path

# Creation of this helper is followed by one explicit trigger commit so the path-filtered validator runs.
ROOT = Path(__file__).resolve().parents[2]
LSM = ROOT / "crates/db-storage-lsm/src/lib.rs"
README = ROOT / "README.md"
ROADMAP = ROOT / "docs/roadmap.md"
DESIGN = ROOT / "docs/design-space.md"
METHOD = ROOT / "docs/amplification-methodology.md"


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one marker, found {count}")
    return text.replace(old, new, 1)


# Preserve the public db-storage-lsm::AmplificationRatio path while sharing the core type.
text = LSM.read_text()
text = replace_once(
    text,
    '''    validate_key, validate_key_value, validate_range_scan, AmplificationInstrumented,
    AmplificationRatio, AmplificationReport, ConcurrencyMode, CrashRecovery, DbError,
''',
    '''    validate_key, validate_key_value, validate_range_scan, AmplificationInstrumented,
    AmplificationReport, ConcurrencyMode, CrashRecovery, DbError,
''',
    "remove private AmplificationRatio import",
)
text = replace_once(
    text,
    '''pub use wal::{RecoveredWalTail, WalVerification};
''',
    '''pub use db_core::AmplificationRatio;
pub use wal::{RecoveredWalTail, WalVerification};
''',
    "preserve LSM AmplificationRatio re-export",
)
LSM.write_text(text)

# README: make the Phase 4 evidence contract visible without claiming device-level fairness yet.
text = README.read_text()
text = replace_once(
    text,
    '''| `db-core` | Binary KV semantics, explicit capabilities, versioned workload model, stable seeded generator, execution and differential harness |
''',
    '''| `db-core` | Binary KV semantics, explicit capabilities, versioned workload model, stable seeded generator, execution/differential harness, experiment compatibility preflight, and common amplification report schema |
''',
    "README core row",
)
text = replace_once(
    text,
    '''| `db-storage-btree` | Common persistent `KvEngine` with fixed 4 KiB checksummed pages, mirrored superblocks, COW `GET`/`PUT`/`DELETE`/`REOPEN`, half-open ordered `range_scan`, split/rebalance/root contraction, reachability-derived page reuse, overflow-backed 4 KiB keys and 1 MiB values, reachable-tree validation, and bounded validated-page caching |
''',
    '''| `db-storage-btree` | Common persistent `KvEngine` with fixed 4 KiB checksummed pages, mirrored superblocks, COW `GET`/`PUT`/`DELETE`/`REOPEN`, half-open ordered `range_scan`, split/rebalance/root contraction, reachability-derived page reuse, overflow-backed 4 KiB keys and 1 MiB values, reachable-tree validation, bounded validated-page caching, and exact structural amplification instrumentation |
''',
    "README btree row",
)
text = replace_once(
    text,
    '''| `db-storage-lsm` | Common persistent `KvEngine` with checksummed segmented WALs, ordered MemTables, indexed/checksummed immutable SSTables, validated embedded Bloom filters, Manifest-v5 L0/L1, tombstone-GC, and SSTable-allocation metadata, crash-published full-set compaction, mirrored `CURRENT`, WAL/SSTable/manifest reclamation, half-open range scans, deterministic compaction differential tests, and exact integer amplification instrumentation |
''',
    '''| `db-storage-lsm` | Common persistent `KvEngine` with checksummed segmented WALs, ordered MemTables, indexed/checksummed immutable SSTables, validated embedded Bloom filters, Manifest-v5 L0/L1, tombstone-GC, and SSTable-allocation metadata, crash-published full-set compaction, mirrored `CURRENT`, WAL/SSTable/manifest reclamation, half-open range scans, deterministic compaction differential tests, and the shared exact amplification report contract |
''',
    "README lsm row",
)
text = replace_once(
    text,
    '''power-loss modeling remain deferred.

The LSM is not an adapter around `db-storage-log`''',
    '''power-loss modeling remain deferred. Process-local B+ tree instrumentation now records explicit
GET/range structural page accesses, acknowledged logical mutation bytes, and synchronized data-page
bytes without counting the PUT/DELETE previous-value lookup as a user GET. Its common amplification
report counts retained committed data pages—including reusable COW history—as primary-structure space.
Hand-computable tests cover a one-leaf COW/reuse trace and an 8 KiB value whose lookup requires exactly
one leaf plus three overflow-page accesses.

The LSM is not an adapter around `db-storage-log`''',
    "README btree instrumentation narrative",
)
text = replace_once(
    text,
    '''comparison participant.

Current common semantics allow empty and arbitrary binary keys/values,''',
    '''comparison participant.

Both persistent candidates now implement `db_core::AmplificationInstrumented` and return the same
`AmplificationReport` shape. That does **not** make their structural read numerators interchangeable:
B+ tree reads carry the explicit unit `btree_page_access`, LSM point reads carry `lsm_sstable_consult`,
and LSM range reads carry `lsm_sstable_version_decoded`. The common capability preflight requires the
logical model, concurrency/persistence/distribution contract, ordered-range support, and key/value limits
to agree while deliberately allowing storage architecture and recovery mechanism to differ. See
`docs/amplification-methodology.md` for exact accounting boundaries. Device I/O, cache-miss attribution,
latency distributions, and controlled-host performance measurements remain later Phase 4 evidence.

Current common semantics allow empty and arbitrary binary keys/values,''',
    "README common amplification narrative",
)
README.write_text(text)

# Roadmap: first Phase 4 foundation is now executable; common counters exist but common trace runner remains.
text = ROADMAP.read_text()
text = replace_once(
    text,
    '''- [ ] Freeze common point/range/write/delete/durability semantics and capability preflight.
- [ ] Implement reproducible point-read, range-scan, sequential-write, random-write, and mixed traces.
- [ ] Generalize validated read/write/space amplification counters into a common cross-engine experiment
  contract. LSM-local hand-computable counters are now proven; B+ tree parity and shared reporting remain.
''',
    '''- [x] Freeze common point/range/write/delete/durability semantics and capability preflight. The
  reusable preflight rejects mismatched logical model, caller/concurrency contract, persistence class,
  distribution mode, ordered-range support, or key/value limits while allowing storage architecture and
  recovery mechanism to remain the experimental variables.
- [ ] Implement reproducible point-read, range-scan, sequential-write, random-write, and mixed traces.
- [ ] Drive the common amplification contract from shared cross-engine traces. B+ tree and LSM now both
  implement `AmplificationInstrumented` and return the same exact report shape; hand-computable tests
  validate B+ tree page-access/data-page/retained-page accounting and the existing LSM consult/version/
  WAL+SSTable accounting. Read work carries an explicit architecture-specific unit so structural counters
  cannot be mislabeled as comparable device I/O. The remaining work is one shared trace runner and archived
  per-engine raw evidence under identical trace inputs.
''',
    "Phase 4 roadmap foundation",
)
ROADMAP.write_text(text)

# Design-space status: both engines have the common evidence surface but no benchmark claim.
text = DESIGN.read_text()
text = replace_once(
    text,
    '''| Binary KV + B+ tree + standalone | Common persistent point + ordered-range engine implemented | `db-storage-btree` implements `KvEngine` for the full 4 KiB-key/1 MiB-value point contract plus bounded half-open ordered scans. Scans traverse internal children in separator order without leaf sibling links and are differentially checked against the memory oracle. A deterministic durable-write fault matrix covers appended/recycled overflow, leaf, and internal pages plus allocation/root superblocks with pre-write, torn-half-write, and post-sync errors, proving old-or-complete-new reopen behavior and safe orphan repair; physical compaction remains deferred |
''',
    '''| Binary KV + B+ tree + standalone | Common persistent semantics + structural amplification evidence implemented | `db-storage-btree` implements the full 4 KiB-key/1 MiB-value point/range contract, deterministic fault recovery, and the common amplification-report trait. Hand-computable traces measure logical validated-page accesses (including cache hits), synchronized data-page writes, and retained committed data-page space including reusable COW history. Read work is explicitly tagged `btree_page_access`; it is not presented as device I/O. Physical compaction, shared generated experiment traces, and controlled-host measurements remain deferred |
''',
    "design-space btree row",
)
text = replace_once(
    text,
    '''| Binary KV + LSM + standalone | Persistent correctness engine with local amplification evidence; not yet a performance candidate | `db-storage-lsm` implements the common point/reopen/range contract with checksummed segmented WALs, ordered MemTables, immutable indexed/checksummed SSTables, mirrored CURRENT, crash-safe WAL rotation/reclamation, SSTable v2 embedded Bloom filters, and Manifest-v5 full-set L0/L1 compaction. The serialized snapshot-free full-set proof point elides tombstones, including a GC-covered zero-SSTable state; v5 also preserves the SSTable allocation frontier across canonical-orphan cleanup so ambiguous ids are not reused. Deterministic fault matrices require complete old-or-new recovery, and deterministic multi-cycle compaction is differentially checked against the memory oracle. Exact integer LSM-local point/range/data-write/sorted-table-space counters are validated on hand-computable traces. Generalized levels, snapshot/replication-aware GC, common cross-engine metrics, controlled-host measurements, and comparable performance evidence remain deferred |
''',
    '''| Binary KV + LSM + standalone | Persistent correctness + common structural amplification evidence; not yet a performance candidate | `db-storage-lsm` implements the common point/reopen/range contract, crash-safe WAL/SSTable/Manifest-v5 full-set compaction, deterministic old-or-new fault recovery, and multi-cycle differential compaction tests. Its proven raw counters now feed the common amplification report: point reads are tagged `lsm_sstable_consult`, range work is tagged `lsm_sstable_version_decoded`, data writes cover WAL records plus flush/compaction SSTable output, and primary structure is authoritative SSTable bytes. Generalized levels, snapshot/replication-aware GC, shared generated experiment traces, device-level telemetry, controlled-host measurements, and comparable performance evidence remain deferred |
''',
    "design-space lsm row",
)
DESIGN.write_text(text)

METHOD.write_text('''# Amplification evidence methodology

This document freezes the first common reporting contract used before any B+ tree versus LSM performance
claim. The goal is reproducible structural evidence, not a synthetic promise that unlike storage engines
perform the same physical I/O operations.

## Common report shape

`db_core::AmplificationReport` stores exact integer numerator/denominator pairs. Ratios are never rounded
inside the engine and a zero denominator is preserved. Both persistent comparison candidates implement
`AmplificationInstrumented`, whose reset operation changes only process-local counters and whose report
operation does not change logical database state.

The report contains four fields:

| Field | Numerator | Denominator |
| --- | --- | --- |
| `point_read` | structural read work, with an explicit `ReadWorkUnit` | successful explicit `GET` calls |
| `range_read` | structural read work, with an explicit `ReadWorkUnit` | logical records returned by successful range scans |
| `data_write_bytes_per_logical_byte` | documented engine data-path bytes written | acknowledged logical mutation bytes |
| `primary_structure_bytes_per_live_byte` | documented retained primary-structure bytes | live logical key + value bytes represented by that structure |

PUT contributes key + value bytes to the logical write denominator. DELETE contributes key bytes even
when the key is missing, because the successful call is still part of the requested mutation trace.
Internal previous-value lookups used to implement PUT/DELETE return values are never counted as explicit
user point reads.

## Read amplification is structural and unit-tagged

A raw structural numerator is meaningful only together with its `ReadWorkUnit`:

- B+ tree point and range reads use `btree_page_access`: one logical access to a validated 4 KiB data page,
  including a page served from the bounded cache. Overflow-key and overflow-value pages count because the
  logical operation must traverse them.
- LSM point reads use `lsm_sstable_consult`: one SSTable considered before its bounds/Bloom/index path
  establishes hit or miss.
- LSM range reads use `lsm_sstable_version_decoded`: one physical SSTable record version decoded while
  resolving newest visible state.

Therefore `4 btree_page_access / GET` and `4 lsm_sstable_consult / GET` are **not** four units of the same
physical resource. The common schema makes this mismatch machine-visible instead of silently calling both
numbers "read I/O amplification". Device bytes read, cache misses, system calls, page-cache behavior, and
hardware counters belong to a later controlled-host measurement layer.

## B+ tree byte accounting

B+ tree data-write bytes count successfully synchronized leaf, internal, key-overflow, and value-overflow
page images produced during the measurement window. Mirrored allocation/root superblock writes are excluded
from this data-path numerator. Failed mutations are not added to the logical denominator; durable page work
from an ambiguous failed operation is likewise not retroactively assigned to a later successful mutation.

Primary-structure bytes are `committed_data_pages * 4096`. This intentionally includes unreachable COW
history still retained in the page file because those bytes remain allocated storage and are available for
later orphan reuse. The two fixed mirrored superblocks are excluded. The live-byte denominator is rebuilt
from the authoritative root without incrementing public read counters.

Hand-computable regression evidence includes a two-key one-leaf file: after COW history creates two retained
data pages, one recycled-leaf overwrite plus one missing DELETE produces exactly 4096 data-write bytes over
four logical mutation bytes. One explicit point lookup touches one page; a full two-row range touches one
page and returns two records. An 8192-byte value occupies three 4048-byte overflow chunks, so point/range
retrieval each requires exactly one leaf plus three overflow page accesses.

## LSM byte accounting

LSM data-write bytes are complete WAL mutation records plus immutable SSTable bytes produced by MemTable
flushes and compaction outputs. Manifest snapshots, mirrored `CURRENT`, filesystem metadata, cache traffic,
and device writeback are excluded. The separate raw counter for compaction-input SSTable bytes remains
available as architecture-specific evidence but is not added again to the write-output numerator.

Primary-structure bytes are authoritative SSTable bytes divided by durable live key + value bytes represented
by those SSTables. Unflushed WAL/MemTable state is excluded from both sides. Existing hand-computable tests
prove WAL framing, flush/output byte totals, first full-set compaction input, `5/3` point SSTable consults,
and `10/9` decoded range versions per logical result on a layered L0/L1 state.

## Capability preflight

`validate_experiment_compatibility` rejects a comparison when engines disagree on the common logical model,
caller/concurrency contract, persistence class, distribution mode, ordered-range capability, or key/value
limits. Storage architecture and crash-recovery mechanism are intentionally allowed to differ: those are the
physical design choices the experiment exists to compare. A range-bearing experiment can additionally
require ordered-range support explicitly.

This preflight does not establish performance fairness by itself. Every future benchmark must still record
the exact trace, engine settings, binary revision, operating system, filesystem/device context, cache state,
and raw counters. No latency or device-level conclusion should be published from the structural ratios in
this document alone.
''')

print("finalized Phase 4 amplification compatibility and documentation")
