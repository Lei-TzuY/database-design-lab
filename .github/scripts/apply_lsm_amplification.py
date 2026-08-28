from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LIB = ROOT / "crates/db-storage-lsm/src/lib.rs"
SST = ROOT / "crates/db-storage-lsm/src/sstable.rs"
TEST = ROOT / "crates/db-storage-lsm/src/instrumentation_tests.rs"


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if text.count(old) != 1:
        raise SystemExit(f"{label}: expected exactly one marker, found {text.count(old)}")
    return text.replace(old, new, 1)


lib = LIB.read_text()

lib = replace_once(
    lib,
    "    pub tombstone_gc_sequence: u64,\n}\n\n/// Read-only verification result for the implemented LSM directory state.\n",
    '''    pub tombstone_gc_sequence: u64,
}

/// Process-local, resettable counters for reproducible LSM amplification experiments.
///
/// These counters deliberately describe the implemented data path rather than pretending to be
/// device-level I/O telemetry. SSTables are fully resident after open, so point/range read counters
/// measure sorted-table work (tables consulted and physical versions decoded). Write counters include
/// WAL mutation records plus immutable SSTable bytes produced by flush and compaction; manifest,
/// CURRENT, filesystem metadata, cache traffic, and device writeback are outside this accounting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct LsmInstrumentation {
    /// Successful explicit `GET` operations.
    pub point_reads: u64,
    /// SSTables consulted by successful explicit `GET` operations.
    pub point_sstable_consults: u64,
    /// Successful explicit range-scan operations, including empty-result scans.
    pub range_scans: u64,
    /// Logical records returned by successful range scans.
    pub range_result_records: u64,
    /// Physical SSTable records decoded while serving successful range scans.
    pub range_sstable_records_decoded: u64,
    /// Successful acknowledged PUT/DELETE mutations.
    pub logical_mutations: u64,
    /// Key plus PUT-value bytes accepted by successful mutations; DELETE contributes key bytes only.
    pub logical_mutation_bytes: u64,
    /// Complete encoded WAL mutation-record bytes written during this measurement window.
    pub wal_record_bytes_written: u64,
    /// Number of immutable MemTable flush SSTables created.
    pub flushes: u64,
    /// SSTable file bytes written by immutable MemTable flushes.
    pub flush_sstable_bytes_written: u64,
    /// Number of full-set compactions started after the L0 trigger fired.
    pub compactions: u64,
    /// Authoritative SSTable bytes consumed as full-set compaction input.
    pub compaction_input_sstable_bytes: u64,
    /// Replacement L1 SSTable bytes written by compaction; table-less GC contributes zero.
    pub compaction_output_sstable_bytes_written: u64,
}

/// Exact integer numerator/denominator pair for an amplification metric.
///
/// A zero denominator is preserved rather than converted to NaN/infinity so experiment code can
/// decide how to render an empty measurement window without losing the raw evidence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct AmplificationRatio {
    /// Raw work/space numerator.
    pub numerator: u64,
    /// Raw logical baseline denominator.
    pub denominator: u64,
}

/// Reproducible amplification report derived from current state plus process-local counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct LsmAmplificationReport {
    /// SSTables consulted per explicit point GET.
    pub point_read_tables_per_get: AmplificationRatio,
    /// Physical SSTable versions decoded per logical range record returned.
    pub range_versions_per_result: AmplificationRatio,
    /// WAL-record + flush-SSTable + compaction-output bytes per acknowledged logical mutation byte.
    pub data_write_bytes_per_logical_byte: AmplificationRatio,
    /// Authoritative SSTable bytes per durable live key+value byte represented by those SSTables.
    pub sorted_table_bytes_per_durable_live_byte: AmplificationRatio,
}

/// Read-only verification result for the implemented LSM directory state.
''',
    "insert instrumentation structs",
)

lib = replace_once(
    lib,
    "    next_manifest_id: u64,\n    next_wal_id: u64,\n    poisoned: bool,\n",
    "    next_manifest_id: u64,\n    next_wal_id: u64,\n    instrumentation: LsmInstrumentation,\n    poisoned: bool,\n",
    "engine instrumentation field",
)

lib = replace_once(
    lib,
    "            next_manifest_id: 2,\n            next_wal_id: 2,\n            poisoned: false,\n",
    "            next_manifest_id: 2,\n            next_wal_id: 2,\n            instrumentation: LsmInstrumentation::default(),\n            poisoned: false,\n",
    "new engine instrumentation init",
)

lib = replace_once(
    lib,
    "            next_manifest_id: checked_next_id(layout.max_manifest_id, \"manifest\")?,\n            next_wal_id: checked_next_id(layout.max_wal_id, \"WAL\")?,\n            version,\n            poisoned: false,\n",
    "            next_manifest_id: checked_next_id(layout.max_manifest_id, \"manifest\")?,\n            next_wal_id: checked_next_id(layout.max_wal_id, \"WAL\")?,\n            version,\n            instrumentation: LsmInstrumentation::default(),\n            poisoned: false,\n",
    "reopen instrumentation init",
)

lib = replace_once(
    lib,
    "    fn current_entry(&self, key: &[u8]) -> Result<Option<VersionedEntry>> {\n        if let Some(entry) = self.memtables.get(key) {\n            return Ok(Some(entry.clone()));\n        }\n        for table in self.tables.iter().rev() {\n            if let Some(entry) = table.get(key)? {\n                return Ok(Some(entry));\n            }\n        }\n        Ok(None)\n    }\n\n    fn current_value(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {\n        Ok(self.current_entry(key)?.and_then(|entry| entry.value))\n    }\n",
    '''    /// Returns a copy of the process-local instrumentation counters.
    #[must_use]
    pub const fn instrumentation(&self) -> LsmInstrumentation {
        self.instrumentation
    }

    /// Resets process-local amplification counters without modifying database state.
    pub fn reset_instrumentation(&mut self) {
        self.instrumentation = LsmInstrumentation::default();
    }

    /// Builds exact raw amplification ratios from the current measurement window and durable SSTables.
    ///
    /// Computing the sorted-table space denominator decodes the authoritative SSTable set in memory but
    /// does not increment read counters. The denominator excludes unflushed WAL/MemTable state so it is
    /// paired with exactly the durable sorted-table bytes in the numerator.
    pub fn amplification_report(&self) -> Result<LsmAmplificationReport> {
        self.ensure_usable()?;
        let authoritative_sstable_bytes = self
            .version
            .tables
            .iter()
            .fold(0_u64, |total, table| total.saturating_add(table.file_bytes));
        let mut durable = BTreeMap::new();
        for table in &self.tables {
            let _ = table.overlay_range(b"", None, &mut durable)?;
        }
        let durable_live_bytes = durable.into_iter().try_fold(0_u64, |total, (key, entry)| {
            let Some(value) = entry.value else {
                return Ok(total);
            };
            let bytes = key.len().checked_add(value.len()).ok_or_else(|| {
                corruption("durable live logical byte count overflowed usize")
            })?;
            let bytes = u64::try_from(bytes)
                .map_err(|_| corruption("durable live logical byte count does not fit u64"))?;
            total
                .checked_add(bytes)
                .ok_or_else(|| corruption("durable live logical byte count overflowed u64"))
        })?;
        let data_write_bytes = self
            .instrumentation
            .wal_record_bytes_written
            .saturating_add(self.instrumentation.flush_sstable_bytes_written)
            .saturating_add(self.instrumentation.compaction_output_sstable_bytes_written);
        Ok(LsmAmplificationReport {
            point_read_tables_per_get: AmplificationRatio {
                numerator: self.instrumentation.point_sstable_consults,
                denominator: self.instrumentation.point_reads,
            },
            range_versions_per_result: AmplificationRatio {
                numerator: self.instrumentation.range_sstable_records_decoded,
                denominator: self.instrumentation.range_result_records,
            },
            data_write_bytes_per_logical_byte: AmplificationRatio {
                numerator: data_write_bytes,
                denominator: self.instrumentation.logical_mutation_bytes,
            },
            sorted_table_bytes_per_durable_live_byte: AmplificationRatio {
                numerator: authoritative_sstable_bytes,
                denominator: durable_live_bytes,
            },
        })
    }

    fn current_entry_with_consults(
        &self,
        key: &[u8],
    ) -> Result<(Option<VersionedEntry>, u64)> {
        if let Some(entry) = self.memtables.get(key) {
            return Ok((Some(entry.clone()), 0));
        }
        let mut consulted = 0_u64;
        for table in self.tables.iter().rev() {
            consulted = consulted.saturating_add(1);
            if let Some(entry) = table.get(key)? {
                return Ok((Some(entry), consulted));
            }
        }
        Ok((None, consulted))
    }

    fn current_entry(&self, key: &[u8]) -> Result<Option<VersionedEntry>> {
        Ok(self.current_entry_with_consults(key)?.0)
    }

    fn current_value(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self.current_entry(key)?.and_then(|entry| entry.value))
    }
''',
    "instrumentation API and current-entry probes",
)

lib = replace_once(
    lib,
    "        let sequence = match append {\n            Ok(sequence) => sequence,\n            Err(error) => {\n                self.poisoned = true;\n                return Err(error);\n            }\n        };\n",
    '''        let sequence = match append {
            Ok(sequence) => sequence,
            Err(error) => {
                self.poisoned = true;
                return Err(error);
            }
        };
        let encoded_record_bytes = wal::RECORD_HEADER_LEN
            .saturating_add(key.len())
            .saturating_add(value.map_or(0, <[u8]>::len));
        self.instrumentation.wal_record_bytes_written = self
            .instrumentation
            .wal_record_bytes_written
            .saturating_add(u64::try_from(encoded_record_bytes).unwrap_or(u64::MAX));
''',
    "wal byte accounting",
)

lib = replace_once(
    lib,
    "        if let Err(error) = self.flush_frozen_memtables() {\n            self.poisoned = true;\n            return Err(error);\n        }\n        Ok(())\n",
    '''        if let Err(error) = self.flush_frozen_memtables() {
            self.poisoned = true;
            return Err(error);
        }
        self.instrumentation.logical_mutations =
            self.instrumentation.logical_mutations.saturating_add(1);
        let logical_bytes = key
            .len()
            .saturating_add(value.map_or(0, <[u8]>::len));
        self.instrumentation.logical_mutation_bytes = self
            .instrumentation
            .logical_mutation_bytes
            .saturating_add(u64::try_from(logical_bytes).unwrap_or(u64::MAX));
        Ok(())
''',
    "logical mutation accounting",
)

lib = replace_once(
    lib,
    "            let table = SsTable::create_new(&self.path, table_id, durable_sequence, &entries)?;\n            let mut descriptors = self.version.tables.clone();\n",
    '''            let table = SsTable::create_new(&self.path, table_id, durable_sequence, &entries)?;
            self.instrumentation.flushes = self.instrumentation.flushes.saturating_add(1);
            self.instrumentation.flush_sstable_bytes_written = self
                .instrumentation
                .flush_sstable_bytes_written
                .saturating_add(table.descriptor().file_bytes);
            let mut descriptors = self.version.tables.clone();
''',
    "flush accounting",
)

lib = replace_once(
    lib,
    "        let mut merged = BTreeMap::new();\n        for table in &self.tables {\n            table.overlay_range(b\"\", None, &mut merged)?;\n        }\n",
    '''        self.instrumentation.compactions = self.instrumentation.compactions.saturating_add(1);
        let input_bytes = self
            .version
            .tables
            .iter()
            .fold(0_u64, |total, table| total.saturating_add(table.file_bytes));
        self.instrumentation.compaction_input_sstable_bytes = self
            .instrumentation
            .compaction_input_sstable_bytes
            .saturating_add(input_bytes);
        let mut merged = BTreeMap::new();
        for table in &self.tables {
            let _ = table.overlay_range(b"", None, &mut merged)?;
        }
''',
    "compaction input accounting",
)

lib = replace_once(
    lib,
    "            let descriptor = table.descriptor().clone();\n            (Some(table), vec![descriptor], next_table_id)\n",
    '''            let descriptor = table.descriptor().clone();
            self.instrumentation.compaction_output_sstable_bytes_written = self
                .instrumentation
                .compaction_output_sstable_bytes_written
                .saturating_add(descriptor.file_bytes);
            (Some(table), vec![descriptor], next_table_id)
''',
    "compaction output accounting",
)

lib = replace_once(
    lib,
    "    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {\n        validate_key(key)?;\n        self.ensure_usable()?;\n        self.current_value(key)\n    }\n",
    '''    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        validate_key(key)?;
        self.ensure_usable()?;
        let (entry, consulted) = self.current_entry_with_consults(key)?;
        self.instrumentation.point_reads = self.instrumentation.point_reads.saturating_add(1);
        self.instrumentation.point_sstable_consults = self
            .instrumentation
            .point_sstable_consults
            .saturating_add(consulted);
        Ok(entry.and_then(|entry| entry.value))
    }
''',
    "point read accounting",
)

old_range = '''    fn range_scan(
        &mut self,
        start: &[u8],
        end: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        validate_range_scan(start, end)?;
        self.ensure_usable()?;
        if limit == 0 || end.is_some_and(|end| end == start) {
            return Ok(Vec::new());
        }

        let mut visible = BTreeMap::new();
        for table in &self.tables {
            table.overlay_range(start, end, &mut visible)?;
        }
        let lower = Bound::Included(start.to_vec());
        let upper = end
            .map(|end| Bound::Excluded(end.to_vec()))
            .unwrap_or(Bound::Unbounded);
        for (key, entry) in self.memtables.visible_state().range((lower, upper)) {
            let replace = visible
                .get(key.as_slice())
                .is_none_or(|current: &VersionedEntry| entry.sequence > current.sequence);
            if replace {
                visible.insert(key.clone(), entry.clone());
            }
        }
        Ok(visible
            .into_iter()
            .filter_map(|(key, entry)| entry.value.map(|value| (key, value)))
            .take(limit)
            .collect())
    }
'''
new_range = '''    fn range_scan(
        &mut self,
        start: &[u8],
        end: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        validate_range_scan(start, end)?;
        self.ensure_usable()?;
        self.instrumentation.range_scans = self.instrumentation.range_scans.saturating_add(1);
        if limit == 0 || end.is_some_and(|end| end == start) {
            return Ok(Vec::new());
        }

        let mut visible = BTreeMap::new();
        let mut decoded_records = 0_u64;
        for table in &self.tables {
            decoded_records = decoded_records
                .saturating_add(table.overlay_range(start, end, &mut visible)?);
        }
        let lower = Bound::Included(start.to_vec());
        let upper = end
            .map(|end| Bound::Excluded(end.to_vec()))
            .unwrap_or(Bound::Unbounded);
        for (key, entry) in self.memtables.visible_state().range((lower, upper)) {
            let replace = visible
                .get(key.as_slice())
                .is_none_or(|current: &VersionedEntry| entry.sequence > current.sequence);
            if replace {
                visible.insert(key.clone(), entry.clone());
            }
        }
        let result: Vec<_> = visible
            .into_iter()
            .filter_map(|(key, entry)| entry.value.map(|value| (key, value)))
            .take(limit)
            .collect();
        self.instrumentation.range_sstable_records_decoded = self
            .instrumentation
            .range_sstable_records_decoded
            .saturating_add(decoded_records);
        self.instrumentation.range_result_records = self
            .instrumentation
            .range_result_records
            .saturating_add(u64::try_from(result.len()).unwrap_or(u64::MAX));
        Ok(result)
    }
'''
lib = replace_once(lib, old_range, new_range, "range read accounting")

lib = replace_once(
    lib,
    "    fn reopen(&mut self) -> Result<()> {\n        self.wal.take();\n        match Self::open_existing(self.path.clone()) {\n            Ok(reopened) => {\n                *self = reopened;\n                Ok(())\n            }\n",
    '''    fn reopen(&mut self) -> Result<()> {
        let instrumentation = self.instrumentation;
        self.wal.take();
        match Self::open_existing(self.path.clone()) {
            Ok(mut reopened) => {
                reopened.instrumentation = instrumentation;
                *self = reopened;
                Ok(())
            }
''',
    "preserve instrumentation across reopen",
)

lib = replace_once(
    lib,
    "#[cfg(test)]\nmod compaction_fault_tests;\n",
    "#[cfg(test)]\nmod compaction_fault_tests;\n#[cfg(test)]\nmod instrumentation_tests;\n",
    "instrumentation test module",
)

LIB.write_text(lib)

sst = SST.read_text()
sst = replace_once(
    sst,
    '''    pub(super) fn overlay_range(
        &self,
        start: &[u8],
        end: Option<&[u8]>,
        visible: &mut BTreeMap<Vec<u8>, VersionedEntry>,
    ) -> Result<()> {
        for entry in &self.index {
''',
    '''    pub(super) fn overlay_range(
        &self,
        start: &[u8],
        end: Option<&[u8]>,
        visible: &mut BTreeMap<Vec<u8>, VersionedEntry>,
    ) -> Result<u64> {
        let mut decoded_records = 0_u64;
        for entry in &self.index {
''',
    "overlay return type",
)
sst = replace_once(
    sst,
    "            let (key, decoded, _) = decode_record(&self.bytes, offset, self.bytes.len())?;\n            let replace = visible\n",
    "            let (key, decoded, _) = decode_record(&self.bytes, offset, self.bytes.len())?;\n            decoded_records = decoded_records.saturating_add(1);\n            let replace = visible\n",
    "overlay decoded record accounting",
)
sst = replace_once(
    sst,
    "        Ok(())\n    }\n\n    #[cfg(test)]\n    pub(super) fn bloom_may_contain",
    "        Ok(decoded_records)\n    }\n\n    #[cfg(test)]\n    pub(super) fn bloom_may_contain",
    "overlay return count",
)
SST.write_text(sst)

TEST.write_text(r'''use std::fs;
use std::path::Path;

use db_core::KvEngine;
use db_storage_memory::MemoryEngine;
use tempfile::tempdir;

use super::wal::RECORD_HEADER_LEN;
use super::{AmplificationRatio, LsmEngine, MUTABLE_MEMTABLE_BYTES_LIMIT};

fn large_value(byte: u8) -> Vec<u8> {
    vec![byte; MUTABLE_MEMTABLE_BYTES_LIMIT / 2 + 1_024]
}

fn fixed_key(index: u8) -> Vec<u8> {
    format!("k{index:03}").into_bytes()
}

fn canonical_sstable_bytes(path: &Path) -> u64 {
    fs::read_dir(path)
        .expect("read engine directory")
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("sst-") && name.ends_with(".sst")
        })
        .map(|entry| entry.metadata().expect("SSTable metadata").len())
        .sum()
}

fn assert_same_state(reference: &mut MemoryEngine, engine: &mut LsmEngine) {
    assert_eq!(
        engine.range_scan(b"", None, 128).expect("LSM full range"),
        reference
            .range_scan(b"", None, 128)
            .expect("oracle full range")
    );
}

fn paired_put(
    reference: &mut MemoryEngine,
    engine: &mut LsmEngine,
    key: &[u8],
    value: &[u8],
) {
    assert_eq!(
        engine.put(key, value).expect("LSM put"),
        reference.put(key, value).expect("oracle put")
    );
}

fn paired_delete(reference: &mut MemoryEngine, engine: &mut LsmEngine, key: &[u8]) {
    assert_eq!(
        engine.delete(key).expect("LSM delete"),
        reference.delete(key).expect("oracle delete")
    );
}

#[test]
fn deterministic_full_set_compactions_match_memory_oracle_across_reopen() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    let mut engine = LsmEngine::create_new(&path).expect("create LSM engine");
    let mut reference = MemoryEngine::new();

    for index in 0_u8..8 {
        paired_put(
            &mut reference,
            &mut engine,
            &fixed_key(index),
            &large_value(0x20 + index),
        );
        if index % 2 == 1 {
            assert_same_state(&mut reference, &mut engine);
        }
    }
    let first = engine.stats().expect("first compacted stats");
    assert_eq!(first.level0_sstables, 0);
    assert_eq!(first.level1_sstables, 1);
    assert_eq!(first.sstable_entries, 8);
    engine.reopen().expect("reopen first compacted version");
    assert_same_state(&mut reference, &mut engine);

    for round in 0_u8..4 {
        paired_delete(&mut reference, &mut engine, &fixed_key(round));
        paired_put(
            &mut reference,
            &mut engine,
            &fixed_key(8 + round),
            &large_value(0x40 + round),
        );
        paired_put(
            &mut reference,
            &mut engine,
            &fixed_key(4 + round),
            &large_value(0x50 + round),
        );
        assert_same_state(&mut reference, &mut engine);
    }

    let second = engine.stats().expect("second compacted stats");
    assert_eq!(second.level0_sstables, 0);
    assert_eq!(second.level1_sstables, 1);
    assert_eq!(second.sstable_entries, 8);
    assert_eq!(second.tombstone_gc_sequence, second.durable_sequence);
    for index in 0_u8..4 {
        assert_eq!(engine.get(&fixed_key(index)).expect("deleted key"), None);
    }
    engine.reopen().expect("reopen second compacted version");
    assert_same_state(&mut reference, &mut engine);
    assert_eq!(LsmEngine::verify(&path).expect("verify").memtables, second);
}

#[test]
fn amplification_counters_match_hand_computable_compaction_and_read_trace() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    let mut engine = LsmEngine::create_new(&path).expect("create LSM engine");
    engine.reset_instrumentation();

    let value_len = large_value(0x11).len();
    let logical_per_put = fixed_key(0).len() + value_len;
    for index in 0_u8..8 {
        engine
            .put(&fixed_key(index), &large_value(0x60 + index))
            .expect("build four L0 flushes and compact");
    }

    let counters = engine.instrumentation();
    assert_eq!(counters.logical_mutations, 8);
    assert_eq!(counters.flushes, 4);
    assert_eq!(counters.compactions, 1);
    assert_eq!(
        counters.logical_mutation_bytes,
        u64::try_from(8 * logical_per_put).expect("logical bytes fit u64")
    );
    assert_eq!(
        counters.wal_record_bytes_written,
        u64::try_from(8 * (RECORD_HEADER_LEN + logical_per_put)).expect("WAL bytes fit u64")
    );
    assert_eq!(
        counters.compaction_input_sstable_bytes,
        counters.flush_sstable_bytes_written,
        "first full-set compaction consumes exactly the four flush outputs"
    );
    let physical_after_compaction = canonical_sstable_bytes(&path);
    assert_eq!(
        counters.compaction_output_sstable_bytes_written,
        physical_after_compaction,
        "only the replacement L1 remains authoritative after cleanup"
    );

    let report = engine.amplification_report().expect("amplification report");
    assert_eq!(
        report.data_write_bytes_per_logical_byte,
        AmplificationRatio {
            numerator: counters
                .wal_record_bytes_written
                .saturating_add(counters.flush_sstable_bytes_written)
                .saturating_add(counters.compaction_output_sstable_bytes_written),
            denominator: counters.logical_mutation_bytes,
        }
    );
    assert_eq!(
        report.sorted_table_bytes_per_durable_live_byte,
        AmplificationRatio {
            numerator: physical_after_compaction,
            denominator: u64::try_from(8 * logical_per_put).expect("durable bytes fit u64"),
        }
    );

    engine.reset_instrumentation();
    engine
        .put(&fixed_key(0), &large_value(0x91))
        .expect("overwrite into newest L0");
    engine
        .put(&fixed_key(8), &large_value(0x92))
        .expect("flush one L0 over L1");
    let layered = engine.stats().expect("layered stats");
    assert_eq!(layered.level0_sstables, 1);
    assert_eq!(layered.level1_sstables, 1);
    assert_eq!(layered.sstable_entries, 10);

    engine.reset_instrumentation();
    assert!(engine.get(&fixed_key(0)).expect("newest L0 key").is_some());
    assert!(engine.get(&fixed_key(1)).expect("older L1 key").is_some());
    assert_eq!(engine.get(b"zzzz").expect("absent key"), None);
    let range = engine.range_scan(b"", None, 128).expect("full layered range");
    assert_eq!(range.len(), 9);

    let counters = engine.instrumentation();
    assert_eq!(counters.point_reads, 3);
    assert_eq!(counters.point_sstable_consults, 5);
    assert_eq!(counters.range_scans, 1);
    assert_eq!(counters.range_sstable_records_decoded, 10);
    assert_eq!(counters.range_result_records, 9);
    let layered_report = engine
        .amplification_report()
        .expect("layered amplification report");
    assert_eq!(
        layered_report.point_read_tables_per_get,
        AmplificationRatio {
            numerator: 5,
            denominator: 3,
        }
    );
    assert_eq!(
        layered_report.range_versions_per_result,
        AmplificationRatio {
            numerator: 10,
            denominator: 9,
        }
    );
    assert_eq!(
        layered_report.sorted_table_bytes_per_durable_live_byte.numerator,
        canonical_sstable_bytes(&path)
    );
    assert_eq!(
        layered_report.sorted_table_bytes_per_durable_live_byte.denominator,
        u64::try_from(9 * logical_per_put).expect("layered durable bytes fit u64")
    );
}

#[test]
fn instrumentation_survives_logical_reopen_and_reset_is_state_neutral() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    let mut engine = LsmEngine::create_new(&path).expect("create LSM engine");
    engine.put(b"a", b"one").expect("put a");
    assert_eq!(engine.get(b"a").expect("get a"), Some(b"one".to_vec()));
    let before = engine.instrumentation();
    assert_eq!(before.logical_mutations, 1);
    assert_eq!(before.point_reads, 1);

    engine.reopen().expect("logical reopen");
    assert_eq!(engine.instrumentation(), before);
    assert_eq!(engine.get(b"a").expect("get after reopen"), Some(b"one".to_vec()));
    assert_eq!(engine.instrumentation().point_reads, 2);

    let logical = engine.range_scan(b"", None, 16).expect("state before reset");
    engine.reset_instrumentation();
    assert_eq!(engine.instrumentation(), Default::default());
    assert_eq!(engine.range_scan(b"", None, 16).expect("state after reset"), logical);
}
''')
