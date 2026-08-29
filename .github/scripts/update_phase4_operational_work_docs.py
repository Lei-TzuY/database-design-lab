from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
README = ROOT / "README.md"
METHOD = ROOT / "docs/amplification-methodology.md"
TRACES = ROOT / "docs/experiment-traces.md"
ROADMAP = ROOT / "docs/roadmap.md"


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one marker, found {count}")
    return text.replace(old, new, 1)


text = METHOD.read_text()
old = '''## Recovery and compaction duration samples

Shared experiment evidence also carries `OperationalTimingReport`. `reopen_ns` records successful same-handle
`REOPEN` wall-clock durations for both persistent candidates. `compaction_stall_ns` records the synchronous LSM
full-set compaction path from trigger entry through publication and obsolete-file reclamation; B+ tree reports
an empty compaction vector. Samples use integer nanoseconds from `std::time::Instant` and are reset after trace
setup together with amplification counters. Failed recovery/compaction attempts are not included in these
success distributions. These raw samples are intentionally ungated in CI: scheduler noise, filesystem/device,
cache state, build profile, and host identity must be archived and pinned before latency comparisons are made.
They are partial operational telemetry, not a complete recovery-cost result: the report does not yet pair each
duration with bytes or records examined, retain failed/excluded attempts, or counterbalance engine execution
order. Evidence archives record the declared environment and cache state, but do not enforce a cache/filesystem
protocol. Those controls and deterministic sample-association tests are required before publishing B+ tree
versus LSM timing distributions.
'''
new = '''## Recovery and compaction operational samples

Shared experiment evidence also carries `OperationalTimingReport`. The original `reopen_ns` and
`compaction_stall_ns` vectors remain as backward-compatible duration projections. New `reopen_samples` and
`compaction_stall_samples` pair the same integer `std::time::Instant` duration with the zero-based measured
trace-step index that triggered it and deterministic data-path work. The runner sets the index immediately
before each measured action and clears it immediately afterward, including error returns; regression tests
pin the emitted REOPEN indices to the exact measured trace positions.

Operational work is architecture-specific and explicitly unit-tagged:

- B+ tree REOPEN uses `btree_page_access`. `units_examined` is the logical validated-data-page accesses already
  performed by `BPlusTree::open` during reachable-tree validation and reuse discovery; `bytes_examined` is that
  count times 4096. Mirrored superblock metadata is excluded and no extra reads are performed for telemetry.
- LSM REOPEN uses `lsm_record_version`. Units are complete active-WAL records plus authoritative SSTable record
  versions. Bytes are the original WAL extent scanned during open—including a structurally recoverable tail
  before truncation—plus authoritative SSTable file bytes. CURRENT, manifest, and directory metadata are
  excluded. `SsTable::open` already reads each authoritative SSTable completely, so reporting adds no I/O.
- LSM full-set compaction uses `lsm_sstable_record_version`. Units and bytes are the authoritative input
  descriptor entry counts and file sizes captured at trigger entry. The sample is appended only after the new
  version is published, mirrored, and obsolete-file reclamation completes. B+ tree reports no compaction
  samples.

The compatibility duration vectors and structured vectors are appended together and tests require their
indices/durations to agree. Successful-sample work accounting is therefore deterministic and trace-associated,
but the roadmap item remains incomplete: failed/excluded recovery or compaction attempts are not retained,
engine execution order is not counterbalanced, and the archive's declared cache/filesystem state is not an
enforced protocol. Scheduler noise, build profile, host identity, cache state, filesystem, and storage device
must still be controlled before timing distributions can support a performance claim.
'''
text = replace_once(text, old, new, "operational methodology")
METHOD.write_text(text)

text = TRACES.read_text()
text = replace_once(
    text,
    '''The proven common measured-outcome vector once, and per-engine capabilities plus the exact common
`AmplificationReport`.
Read-work units remain architecture-specific (`btree_page_access`, `lsm_sstable_consult`, and
'''.replace("The proven", "proven"),
    '''proven common measured-outcome vector once, and per-engine capabilities plus the exact common
`AmplificationReport` and operational report. Successful REOPEN/compaction samples carry the exact measured
step index plus deterministic bytes/record-or-page work without issuing extra measurement I/O.
Read-work units remain architecture-specific (`btree_page_access`, `lsm_sstable_consult`, and
''',
    "trace comparison evidence",
)
text = replace_once(
    text,
    '''This runner establishes canonical bounded logical inputs, explicit setup/measurement boundaries, lockstep
setup/measured outcome equality, and shared structural amplification reporting. It does **not** establish a
fair latency benchmark by itself. Complete recovery-work accounting, failed/excluded samples, counterbalanced
engine order, a cache/filesystem protocol, and controlled-host pinning remain separate Phase 4 work.
''',
    '''This runner establishes canonical bounded logical inputs, explicit setup/measurement boundaries, lockstep
setup/measured outcome equality, shared structural amplification reporting, and deterministic measured-step
association for successful recovery/compaction work samples. It does **not** establish a fair latency benchmark
by itself. Failed/excluded samples, counterbalanced engine order, an enforced cache/filesystem protocol, and
controlled-host pinning remain separate Phase 4 work.
''',
    "trace scope boundary",
)
TRACES.write_text(text)

text = ROADMAP.read_text()
text = replace_once(
    text,
    '''- [ ] Complete recovery-cost and compaction-stall distributions. Both engines expose successful measured
  `REOPEN` nanoseconds and the LSM exposes successful synchronous full-set compaction nanoseconds, but this is
  only partial telemetry. Recovery bytes/records examined, deterministic sample-association tests,
  failed/excluded attempts, counterbalanced engine order, and a cache/filesystem protocol are still required
  before this methodology or roadmap item is complete.
''',
    '''- [ ] Complete recovery-cost and compaction-stall distributions. Successful samples now pair duration with
  the exact measured trace-step index and deterministic data-path work: B+ tree reopen page accesses/bytes,
  LSM reopen WAL+SSTable record versions/bytes, and LSM full-set compaction input record versions/bytes. Tests
  pin trace association and the legacy raw-nanosecond projections. Failed/excluded attempts, counterbalanced
  engine order, and an enforced cache/filesystem protocol are still required before this item is complete.
''',
    "roadmap operational evidence",
)
ROADMAP.write_text(text)

text = README.read_text()
text = replace_once(
    text,
    '''`experiment-compare` proves identical setup and measured outcomes in lockstep before archiving both
engines' amplification evidence; `experiment-archive` adds a create-new raw evidence directory plus an
explicit environment manifest.
''',
    '''`experiment-compare` proves identical setup and measured outcomes in lockstep before archiving both
engines' amplification evidence. Successful REOPEN/LSM-compaction timings additionally carry their exact
measured-step index and deterministic page/record plus data-path-byte work while retaining the original raw
nanosecond vectors for compatibility; these samples are still not controlled-host performance claims.
`experiment-archive` adds a create-new raw evidence directory plus an explicit environment manifest.
''',
    "README operational evidence",
)
README.write_text(text)

print("updated Phase 4 operational work methodology")
