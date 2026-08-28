from pathlib import Path

path = Path("crates/db-storage-lsm/src/lib.rs")
text = path.read_text()


def replace_once(old: str, new: str) -> None:
    global text
    if old not in text:
        raise SystemExit(f"missing expected block:\n{old[:200]}")
    text = text.replace(old, new, 1)

replace_once(
    "use std::collections::BTreeMap;",
    "use std::collections::{BTreeMap, BTreeSet};",
)
replace_once(
    "use wal::{MutationKind, Wal, WAL_FILE_NAME};",
    "use wal::{\n    file_name as wal_file_name, MutationKind, Wal, INITIAL_FIRST_SEQUENCE, INITIAL_WAL_ID,\n    WAL_FILE_NAME,\n};",
)
replace_once(
'''    /// Complete mutation records retained in the WAL, including already-flushed history.\n    pub wal_records: u64,''',
'''    /// Complete mutation records retained in the authoritative active WAL segment.\n    pub wal_records: u64,\n    /// Authoritative WAL segment id selected by the manifest.\n    pub active_wal_id: u64,\n    /// First sequence encoded by the authoritative active WAL segment.\n    pub active_wal_first_sequence: u64,''',
)
replace_once(
'''    next_table_id: u64,\n    next_manifest_id: u64,\n    poisoned: bool,''',
'''    next_table_id: u64,\n    next_manifest_id: u64,\n    next_wal_id: u64,\n    poisoned: bool,''',
)
replace_once(
'''    fn initialize_new(path: PathBuf) -> Result<Self> {\n        let wal = Wal::create_new(&path.join(WAL_FILE_NAME))?;\n        let version = manifest::create_initial(&path)?;\n        Ok(Self {\n            path,\n            wal: Some(wal),\n            memtables: MemTableSet::new(MUTABLE_MEMTABLE_BYTES_LIMIT)?,\n            tables: Vec::new(),\n            version,\n            next_table_id: 1,\n            next_manifest_id: 2,\n            poisoned: false,\n        })\n    }''',
'''    fn initialize_new(path: PathBuf) -> Result<Self> {\n        let wal = Wal::create_new(\n            &path.join(wal_file_name(INITIAL_WAL_ID)),\n            INITIAL_WAL_ID,\n            INITIAL_FIRST_SEQUENCE,\n        )?;\n        let version =\n            manifest::create_initial(&path, INITIAL_WAL_ID, INITIAL_FIRST_SEQUENCE)?;\n        Ok(Self {\n            path,\n            wal: Some(wal),\n            memtables: MemTableSet::new(MUTABLE_MEMTABLE_BYTES_LIMIT)?,\n            tables: Vec::new(),\n            version,\n            next_table_id: 1,\n            next_manifest_id: 2,\n            next_wal_id: 2,\n            poisoned: false,\n        })\n    }''',
)
legacy = '''            VersionSet {\n                current_generation: 0,\n                manifest_id: 0,\n                durable_sequence: 0,\n                tables: Vec::new(),\n            }'''
legacy_new = '''            VersionSet {\n                current_generation: 0,\n                manifest_id: 0,\n                durable_sequence: 0,\n                wal_id: INITIAL_WAL_ID,\n                wal_first_sequence: INITIAL_FIRST_SEQUENCE,\n                tables: Vec::new(),\n            }'''
if text.count(legacy) != 2:
    raise SystemExit(f"expected two legacy VersionSet blocks, found {text.count(legacy)}")
text = text.replace(legacy, legacy_new)

replace_once(
'''        let mut memtables = MemTableSet::new(MUTABLE_MEMTABLE_BYTES_LIMIT)?;\n        let durable_sequence = version.durable_sequence;\n        let wal = Wal::open(&layout.wal_path, |mutation| {\n            if mutation.sequence > durable_sequence {\n                memtables.apply(mutation.sequence, mutation.key, mutation.value)?;\n            }\n            Ok(())\n        })?;\n        if durable_sequence > wal.record_count() {\n            return Err(corruption(format!(\n                "manifest durable sequence {durable_sequence} exceeds WAL record count {}",\n                wal.record_count()\n            )));\n        }''',
'''        if !layout.wal_ids.contains(&version.wal_id) {\n            return Err(corruption(format!(\n                "manifest references missing WAL segment {}",\n                version.wal_id\n            )));\n        }\n        let wal_path = path.join(wal_file_name(version.wal_id));\n        let mut memtables = MemTableSet::new(MUTABLE_MEMTABLE_BYTES_LIMIT)?;\n        let durable_sequence = version.durable_sequence;\n        let wal = Wal::open(\n            &wal_path,\n            version.wal_id,\n            version.wal_first_sequence,\n            |mutation| {\n                if mutation.sequence > durable_sequence {\n                    memtables.apply(mutation.sequence, mutation.key, mutation.value)?;\n                }\n                Ok(())\n            },\n        )?;\n        if durable_sequence >= wal.next_sequence() {\n            return Err(corruption(format!(\n                "manifest durable sequence {durable_sequence} is not below WAL next sequence {}",\n                wal.next_sequence()\n            )));\n        }''',
)
replace_once(
'''            next_table_id: checked_next_id(layout.max_table_id, "SSTable")?,\n            next_manifest_id: checked_next_id(layout.max_manifest_id, "manifest")?,\n            version,\n            poisoned: false,''',
'''            next_table_id: checked_next_id(layout.max_table_id, "SSTable")?,\n            next_manifest_id: checked_next_id(layout.max_manifest_id, "manifest")?,\n            next_wal_id: checked_next_id(layout.max_wal_id, "WAL")?,\n            version,\n            poisoned: false,''',
)
replace_once(
'''        let mut memtables = MemTableSet::new(MUTABLE_MEMTABLE_BYTES_LIMIT)?;\n        let durable_sequence = version.durable_sequence;\n        let wal = Wal::verify(&layout.wal_path, |mutation| {\n            if mutation.sequence > durable_sequence {\n                memtables.apply(mutation.sequence, mutation.key, mutation.value)?;\n            }\n            Ok(())\n        })?;''',
'''        if !layout.wal_ids.contains(&version.wal_id) {\n            return Err(corruption(format!(\n                "manifest references missing WAL segment {}",\n                version.wal_id\n            )));\n        }\n        let wal_path = path.join(wal_file_name(version.wal_id));\n        let mut memtables = MemTableSet::new(MUTABLE_MEMTABLE_BYTES_LIMIT)?;\n        let durable_sequence = version.durable_sequence;\n        let wal = Wal::verify(\n            &wal_path,\n            version.wal_id,\n            version.wal_first_sequence,\n            |mutation| {\n                if mutation.sequence > durable_sequence {\n                    memtables.apply(mutation.sequence, mutation.key, mutation.value)?;\n                }\n                Ok(())\n            },\n        )?;''',
)
replace_once(
'''            memtables: LsmStats {\n                wal_records: wal.record_count,''',
'''            memtables: LsmStats {\n                wal_records: wal.record_count,\n                active_wal_id: wal.wal_id,\n                active_wal_first_sequence: wal.first_sequence,''',
)
replace_once(
'''        Ok(LsmStats {\n            wal_records: wal.record_count(),''',
'''        Ok(LsmStats {\n            wal_records: wal.record_count(),\n            active_wal_id: wal.wal_id(),\n            active_wal_first_sequence: wal.first_sequence(),''',
)
replace_once(
'''            if self.version.manifest_id == 0 {\n                self.version = manifest::create_initial(&self.path)?;\n                self.next_manifest_id = 2;\n            }''',
'''            if self.version.manifest_id == 0 {\n                let wal = self.wal.as_ref().ok_or(DbError::Poisoned)?;\n                self.version = manifest::create_initial(\n                    &self.path,\n                    wal.wal_id(),\n                    wal.first_sequence(),\n                )?;\n                self.next_manifest_id = 2;\n            }''',
)
replace_once(
'''            let next_version = manifest::install(\n                &self.path,\n                &self.version,\n                manifest_id,\n                durable_sequence,\n                descriptors,\n            )?;''',
'''            let next_version = manifest::install(\n                &self.path,\n                &self.version,\n                manifest_id,\n                durable_sequence,\n                descriptors,\n                self.version.wal_id,\n                self.version.wal_first_sequence,\n            )?;''',
)
replace_once(
'''            self.memtables.retire_oldest_immutable()?;\n        }\n        Ok(())\n    }\n\n    fn ensure_usable(&self) -> Result<()> {''',
'''            self.memtables.retire_oldest_immutable()?;\n        }\n        self.maybe_rotate_wal()?;\n        Ok(())\n    }\n\n    fn maybe_rotate_wal(&mut self) -> Result<()> {\n        let Some(first_sequence) = self.version.durable_sequence.checked_add(1) else {\n            return Ok(());\n        };\n        let wal = self.wal.as_ref().ok_or(DbError::Poisoned)?;\n        if wal.record_count() == 0\n            || wal.next_sequence() != first_sequence\n            || wal.first_sequence() == first_sequence\n        {\n            return Ok(());\n        }\n\n        let old_wal_id = wal.wal_id();\n        let new_wal_id = self.next_wal_id;\n        let new_manifest_id = self.next_manifest_id;\n        let following_wal_id = checked_next_id(new_wal_id, "WAL")?;\n        let following_manifest_id = checked_next_id(new_manifest_id, "manifest")?;\n        let new_wal = Wal::create_new(\n            &self.path.join(wal_file_name(new_wal_id)),\n            new_wal_id,\n            first_sequence,\n        )?;\n        let rotated = manifest::install(\n            &self.path,\n            &self.version,\n            new_manifest_id,\n            self.version.durable_sequence,\n            self.version.tables.clone(),\n            new_wal_id,\n            first_sequence,\n        )?;\n        let mirrored = manifest::mirror_current(&self.path, &rotated)?;\n\n        let old_wal = self.wal.replace(new_wal).ok_or(DbError::Poisoned)?;\n        self.version = mirrored;\n        self.next_wal_id = following_wal_id;\n        self.next_manifest_id = following_manifest_id;\n        drop(old_wal);\n        self.reclaim_obsolete_wals(new_wal_id);\n        debug_assert_ne!(old_wal_id, new_wal_id);\n        Ok(())\n    }\n\n    fn reclaim_obsolete_wals(&self, active_wal_id: u64) {\n        let Ok(entries) = fs::read_dir(&self.path) else {\n            return;\n        };\n        for entry in entries.flatten() {\n            let name = entry.file_name();\n            let text = name.to_string_lossy();\n            let Some(wal_id) = parse_numbered_name(&text, "wal-", ".log") else {\n                continue;\n            };\n            if wal_id != active_wal_id {\n                let _ = fs::remove_file(entry.path());\n            }\n        }\n    }\n\n    fn ensure_usable(&self) -> Result<()> {''',
)
replace_once(
    '            name: "lsm-sstable-manifest-v1",',
    '            name: "lsm-segmented-wal-v2",',
)

start = text.index("struct Layout {")
end = text.index("fn parse_numbered_name", start)
layout = '''struct Layout {\n    wal_ids: BTreeSet<u64>,\n    max_wal_id: u64,\n    max_table_id: u64,\n    max_manifest_id: u64,\n    has_version_set: bool,\n}\n\nfn validate_layout(path: &Path) -> Result<Layout> {\n    let metadata = fs::metadata(path)?;\n    if !metadata.is_dir() {\n        return Err(corruption("LSM engine path is not a directory"));\n    }\n\n    let current_name = OsStr::new(CURRENT_FILE_NAME);\n    let mut wal_ids = BTreeSet::new();\n    let mut found_current = false;\n    let mut max_wal_id = 0_u64;\n    let mut max_table_id = 0_u64;\n    let mut max_manifest_id = 0_u64;\n\n    for entry in fs::read_dir(path)? {\n        let entry = entry?;\n        let file_type = entry.file_type()?;\n        if !file_type.is_file() {\n            return Err(corruption(format!(\n                "LSM directory entry is not a regular file: {}",\n                entry.file_name().to_string_lossy()\n            )));\n        }\n        let name = entry.file_name();\n        if name == current_name {\n            if found_current {\n                return Err(corruption(\n                    "LSM directory contains duplicate CURRENT entries",\n                ));\n            }\n            found_current = true;\n            continue;\n        }\n        let text = name.to_string_lossy();\n        if let Some(id) = parse_numbered_name(&text, "wal-", ".log") {\n            wal_ids.insert(id);\n            max_wal_id = max_wal_id.max(id);\n            continue;\n        }\n        if let Some(id) = parse_numbered_name(&text, "MANIFEST-", "") {\n            max_manifest_id = max_manifest_id.max(id);\n            continue;\n        }\n        if let Some(id) = parse_numbered_name(&text, "sst-", ".sst") {\n            max_table_id = max_table_id.max(id);\n            continue;\n        }\n        return Err(corruption(format!(\n            "unknown file in LSM directory: {text}"\n        )));\n    }\n\n    if wal_ids.is_empty() {\n        return Err(corruption("LSM directory contains no canonical WAL segment"));\n    }\n    let has_version_set = match (found_current, max_manifest_id != 0) {\n        (true, true) => true,\n        (false, false)\n            if max_table_id == 0\n                && wal_ids.len() == 1\n                && wal_ids.contains(&INITIAL_WAL_ID) =>\n        {\n            false\n        }\n        (false, false) => {\n            return Err(corruption(\n                "legacy WAL-only layout must contain exactly wal-0000000000000001.log",\n            ));\n        }\n        (false, true) => {\n            return Err(corruption(\n                "LSM directory has manifest snapshots but is missing CURRENT",\n            ));\n        }\n        (true, false) => {\n            return Err(corruption(\n                "LSM directory has CURRENT but no manifest snapshot",\n            ));\n        }\n    };\n\n    Ok(Layout {\n        wal_ids,\n        max_wal_id,\n        max_table_id,\n        max_manifest_id,\n        has_version_set,\n    })\n}\n\n'''
text = text[:start] + layout + text[end:]

replace_once(
'''#[cfg(test)]\nmod sstable_tests;\n#[cfg(test)]\nmod tests;''',
'''#[cfg(test)]\nmod sstable_tests;\n#[cfg(test)]\nmod tests;\n#[cfg(test)]\nmod wal_rotation_tests;''',
)

path.write_text(text)
