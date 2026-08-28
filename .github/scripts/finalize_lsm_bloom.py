from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    if old not in text:
        if new in text:
            return
        raise SystemExit(f"missing block in {path}:\n{old[:300]}")
    path.write_text(text.replace(old, new, 1))

# Harden error-offset arithmetic and lock the deterministic false-positive corpus.
bloom = Path("crates/db-storage-lsm/src/bloom.rs")
text = bloom.read_text()
old = '''        let expected_crc = read_u32(&bytes[payload_end..expected_len]);
        if crc32fast::hash(&bytes[..payload_end]) != expected_crc {
            return Err(corruption(
                offset + u64::try_from(payload_end).unwrap_or(u64::MAX),
                "Bloom section checksum mismatch",
            ));
        }
'''
new = '''        let expected_crc = read_u32(&bytes[payload_end..expected_len]);
        if crc32fast::hash(&bytes[..payload_end]) != expected_crc {
            let checksum_offset = u64::try_from(payload_end)
                .ok()
                .and_then(|delta| offset.checked_add(delta))
                .ok_or_else(|| corruption(offset, "Bloom checksum offset overflowed u64"))?;
            return Err(corruption(
                checksum_offset,
                "Bloom section checksum mismatch",
            ));
        }
'''
if old in text:
    text = text.replace(old, new, 1)
elif new not in text:
    raise SystemExit("Bloom checksum block not found")
old = '''        assert!(
            false_positives * 100 < ABSENT * 2,
            "deterministic Bloom false-positive rate exceeded 2%: {false_positives}/{ABSENT}"
        );
'''
new = '''        assert_eq!(
            false_positives, 424,
            "stable hash/filter semantics changed; review the on-disk Bloom format before updating this fixture"
        );
        assert!(
            false_positives * 100 < ABSENT * 2,
            "deterministic Bloom false-positive rate exceeded 2%: {false_positives}/{ABSENT}"
        );
'''
if old in text:
    text = text.replace(old, new, 1)
elif new not in text:
    raise SystemExit("Bloom FPR assertion block not found")
bloom.write_text(text)

# Exercise corruption through the real engine/manifest path, not only the codec unit test.
tests = Path("crates/db-storage-lsm/src/sstable_tests.rs")
text = tests.read_text()
marker = "fn referenced_sstable_bloom_corruption_fails_closed()"
if marker not in text:
    insert_before = '''#[test]
fn torn_latest_current_slot_after_rotation_uses_same_manifest_and_reclaimed_wal() {'''
    addition = '''#[test]
fn referenced_sstable_bloom_corruption_fails_closed() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    {
        let mut engine = LsmEngine::create_new(&path).expect("create LSM engine");
        engine.put(b"a", &large_value(0x35)).expect("put a");
        engine
            .put(b"b", &large_value(0x36))
            .expect("put b and flush Bloom-backed SSTable");
        assert_eq!(engine.stats().expect("stats").sstables, 1);
    }

    // SSTable v2: 64-byte file header + 40-byte Bloom header, so byte 105 lies in the bit payload.
    flip_byte(&numbered_file(&path, "sst-", ".sst", 1), 105);
    let error = LsmEngine::open(&path).expect_err("Bloom corruption must fail closed");
    assert!(error.to_string().contains("corrupt"));
    let verify_error = LsmEngine::verify(&path).expect_err("verify must reject Bloom corruption");
    assert!(verify_error.to_string().contains("corrupt"));
}

#[test]
fn torn_latest_current_slot_after_rotation_uses_same_manifest_and_reclaimed_wal() {'''
    if insert_before not in text:
        raise SystemExit("SSTable test insertion point missing")
    text = text.replace(insert_before, addition, 1)
    tests.write_text(text)

# README status.
readme = Path("README.md")
replace_once(
    readme,
    '''| `db-storage-lsm` | Common persistent `KvEngine` with checksummed segmented WALs, ordered MemTables, indexed/checksummed immutable SSTables, immutable manifest snapshots, mirrored `CURRENT` publication, crash-safe WAL rotation/reclamation, tombstones, and half-open range scans; no Bloom filters, levels, or compaction yet |''',
    '''| `db-storage-lsm` | Common persistent `KvEngine` with checksummed segmented WALs, ordered MemTables, indexed/checksummed immutable SSTables, validated embedded Bloom filters, immutable manifest snapshots, mirrored `CURRENT` publication, crash-safe WAL rotation/reclamation, tombstones, and half-open range scans; no levels or compaction yet |''',
)
replace_once(
    readme,
    '''There are still no Bloom filters, levels, or compaction, so this remains correctness/recovery evidence—not
yet a fair B+ tree performance comparison participant.''',
    '''New SSTables use format v2 with a checksummed 10-bits/key, 7-probe Bloom section embedded in the same
immutable file; v1 SSTables remain readable. Open validates every indexed key as Bloom-positive before
point reads may use a negative filter result to skip an SSTable, so the probabilistic structure cannot
silently introduce a false negative. Levels and compaction remain absent, so this is still not a fair
B+ tree performance comparison participant.''',
)
replace_once(
    readme,
    '''multi-process writers, LSM Bloom filters/levels/compaction, replication, SQL, MVCC, Raft, graph,
time-series, and columnar execution are not implemented.''',
    '''multi-process writers, LSM levels/compaction, replication, SQL, MVCC, Raft, graph, time-series,
and columnar execution are not implemented.''',
)
replace_once(
    readme,
    '''segments removed. Unknown entries, identity mismatches, sequence gaps, absurd lengths, unexplained tails,
and complete checksum failures fail closed.''',
    '''segments removed. SSTable v2 embeds its Bloom filter inside the same synchronized immutable file; the
filter has its own checksummed parameter/payload encoding and is also covered by the SSTable whole-file
checksum. Unknown entries, identity mismatches, sequence gaps, absurd lengths, unexplained tails, and
complete checksum failures fail closed.''',
)

# Exact persistent format documentation.
fmt = Path("docs/lsm-sstable-manifest-format.md")
replace_once(
    fmt,
    '''version set to a new empty WAL and reclaim older segments only after both CURRENT mirrors move. Bloom
filters, levels, compaction, and tombstone dropping remain outside this version.''',
    '''version set to a new empty WAL and reclaim older segments only after both CURRENT mirrors move. SSTable
v2 now embeds a validated Bloom filter; levels, compaction, and tombstone dropping remain outside this
version.''',
)
start = fmt.read_text().index("## SSTable v1\n")
end = fmt.read_text().index("## Immutable manifest snapshots\n", start)
section = '''## SSTable v1 and v2

Each `sst-%016d.sst` is immutable once created. SSTable v1 remains readable and consists of a 64-byte
header, sorted data records, a complete sorted index, and a 64-byte footer. New files are SSTable v2:
the same 64-byte header is followed by one canonical Bloom section, then data records, index, and footer.
The header's existing `data_offset` identifies the exact end of the Bloom section, so no sidecar file or
new manifest publication object is introduced. The complete SSTable is still written with `create_new`,
`write_all`, and `sync_all` before any manifest may reference it.

The header contains magic `DBLSMSST`, format version (`1` or `2`), table id, entry count,
data/index/footer offsets, reserved zero bytes, and a header CRC-32. Version 1 requires `data_offset = 64`.
Version 2 requires `data_offset > 64` and interprets bytes `[64, data_offset)` as exactly one Bloom
section. The footer must use the same SSTable version as the header. Existing record and index encodings
remain unchanged: each data record contains magic `SSTR`, record version, PUT or DELETE kind, sequence,
bounded key/value lengths, header CRC, key/value bytes, and record CRC; the full index stores each key,
kind, sequence, physical record offset, and its own checksum.

### Bloom section v1

The embedded Bloom filter is deterministic and is built over **every SSTable key**, including keys whose
latest entry is a tombstone. This is required because a false negative for a tombstone could otherwise
resurrect an older value from another table. The current canonical configuration is 10 bits per key
(minimum 64 bits), byte-rounded, with 7 double-hash probes. The hash algorithm id `1` denotes the stable
seeded 64-bit FNV/mixing routine implemented by this repository; Rust's process-dependent
`DefaultHasher` is intentionally not part of the persistent format.

| Offset | Bytes | Meaning | Validation |
| ---: | ---: | --- | --- |
| 0 | 8 | magic `DBLSMBLM` | exact match |
| 8 | 2 | Bloom format version | `1` |
| 10 | 2 | header length | `40` |
| 12 | 1 | hash algorithm id | `1` |
| 13 | 1 | probe count | `7` |
| 14 | 2 | flags | zero |
| 16 | 8 | bit count | exactly `max(keys * 10, 64)`, rounded to 8 bits |
| 24 | 8 | key count | exactly the SSTable entry count |
| 32 | 4 | payload bytes | exactly `bit_count / 8` |
| 36 | 4 | header CRC-32 | bytes 0–35 |
| 40 | variable | packed bit array | exact declared extent |
| tail | 4 | section CRC-32 | header + bit payload |

The SSTable's pre-footer whole-file CRC independently covers this entire Bloom section as well. Any bad
magic/version/parameter, noncanonical extent, key-count disagreement, header/section checksum failure,
or outer SSTable checksum failure is corruption.

Bloom results are never trusted before structural validation. On open, the engine first validates the
full SSTable records/index and then requires **every indexed key** to be Bloom-positive. A false negative
therefore fails closed rather than becoming a missing-key answer. Only after that proof may a point
`GET` skip a table when the key is outside the manifest bounds or the Bloom filter says negative. Range
scans do not consult Bloom filters.

For the frozen configuration, the standard independent-hash approximation
`(1 - exp(-7 / 10))^7` is about 0.82%. The deterministic regression inserts 10,000 fixed binary keys and
queries 50,000 disjoint fixed keys; the committed hash/filter semantics produce exactly 424 false
positives (0.848%) and are additionally gated below 2%. This is a reproducible correctness/configuration
fixture, not a production workload or performance claim.

Opening still validates that every index entry exactly describes the corresponding data record, both
sections have identical strictly increasing keys, header/footer/manifest metadata agree, no entry
sequence exceeds the durable watermark, and the manifest key bounds match the index. The implementation
keeps validated file bytes and the full index resident in memory; that correctness-first choice is not
yet realistic read-amplification evidence.

'''
text = fmt.read_text()
fmt.write_text(text[:start] + section + text[end:])
replace_once(
    fmt,
    '''Point reads search mutable/frozen MemTables first and then authoritative SSTables newest-first. Ordered
range scans merge sequence-tagged SSTable state and the active WAL/MemTable tail, keep the newest version
of each key, remove tombstones, and apply the common half-open bounds/limit. Compaction is still required''',
    '''Point reads search mutable/frozen MemTables first and then authoritative SSTables newest-first. For SSTable
v2, manifest key bounds and the validated Bloom filter can reject a point lookup before index/data record
decoding. Ordered range scans ignore Bloom filters, merge sequence-tagged SSTable state and the active
WAL/MemTable tail, keep the newest version of each key, remove tombstones, and apply the common half-open
bounds/limit. Compaction is still required''',
)
replace_once(
    fmt,
    '''Bloom filters, leveled placement, overlap rules, compaction selection, crash-safe compaction publication,
obsolete SSTable/manifest deletion, tombstone elision, block/cache design, and amplification
instrumentation remain separate evidence milestones.''',
    '''Leveled placement, overlap rules, compaction selection, crash-safe compaction publication, obsolete
SSTable/manifest deletion, tombstone elision, block/cache design, and amplification instrumentation remain
separate evidence milestones. Bloom filtering is now part of SSTable v2.''',
)

# WAL doc only needs its dependency/deferred wording corrected.
wal_doc = Path("docs/lsm-wal-format.md")
replace_once(
    wal_doc,
    '''protocol are specified with `docs/lsm-sstable-manifest-format.md`. Bloom filters, levels, and compaction
remain deferred.''',
    '''protocol and SSTable v2 Bloom filters are specified with `docs/lsm-sstable-manifest-format.md`. Levels
and compaction remain deferred.''',
)
replace_once(
    wal_doc,
    '''The following still require focused design plus executable evidence: Bloom filters, block/cache layout,
levels and overlap policy,''',
    '''The following still require focused design plus executable evidence: block/cache layout, levels and
overlap policy,''',
)

# Roadmap evidence.
roadmap = Path("docs/roadmap.md")
replace_once(
    roadmap,
    '''- [ ] Add Bloom filters with measured false-positive configuration.''',
    '''- [x] Add embedded checksummed SSTable v2 Bloom filters with 10 bits/key and 7 deterministic probes.
  SSTable v1 remains readable; v2 open validates every indexed key as filter-positive before point reads
  may trust a negative result. A fixed 10,000-key / 50,000-absent-key corpus produces 424 false positives
  (0.848%) and remains gated below 2%; corruption and tombstone-key coverage are tested explicitly.''',
)

# Design-space status was stale before this slice; bring it up to the actual implementation boundary.
design = Path("docs/design-space.md")
replace_once(
    design,
    '''| Binary KV + LSM + standalone | Executable WAL/MemTable foundation; not a performance candidate | `db-storage-lsm` implements the common point/reopen/range contract with its own versioned checksummed WAL and sequence-tagged mutable/immutable ordered MemTables. Deterministic differential, prefix-recovery, corruption, tombstone, boundary-size, and reopen tests exist. SSTables, manifests, Bloom filters, levels, compaction, WAL reclamation, and comparable performance evidence do not |''',
    '''| Binary KV + LSM + standalone | Persistent correctness engine; not yet a performance candidate | `db-storage-lsm` implements the common point/reopen/range contract with checksummed segmented WALs, ordered MemTables, immutable indexed/checksummed SSTables, immutable manifests + mirrored CURRENT, crash-safe WAL rotation/reclamation, and SSTable v2 embedded Bloom filters. v1 SSTables remain readable; Bloom negatives are trusted only after full-index no-false-negative validation. Levels, compaction, obsolete-SSTable deletion, safe tombstone dropping, and comparable performance evidence remain deferred |''',
)
replace_once(
    design,
    '''The LSM foundation has the same single-owner restriction. It uses one portable regular WAL file inside
an engine directory and rejects every undeclared directory entry; parent-directory durability at initial
creation is not provided by a portable standard-library primitive.''',
    '''The LSM has the same single-owner restriction. It uses canonical numbered WAL segments selected by the
manifest, immutable SSTables (v2 embedding a Bloom filter), and mirrored CURRENT publication inside one
engine directory; undeclared directory entries fail closed. Parent-directory durability at initial file
creation is not provided by a portable standard-library primitive.''',
)
