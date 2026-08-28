from pathlib import Path

manifest = Path("crates/db-storage-lsm/src/manifest.rs")
text = manifest.read_text()
text = text.replace("use std::path::{Path, PathBuf};", "use std::path::Path;")
old = "return Err(DbError::UnsupportedVersion(version));"
repls = [
    '''return Err(DbError::UnsupportedVersion {\n            format: "LSM CURRENT",\n            found: u64::from(version),\n            supported: u64::from(FORMAT_VERSION),\n        });''',
    '''return Err(DbError::UnsupportedVersion {\n            format: "LSM manifest",\n            found: u64::from(version),\n            supported: u64::from(FORMAT_VERSION),\n        });''',
]
for repl in repls:
    if old not in text:
        raise SystemExit("missing manifest UnsupportedVersion site")
    text = text.replace(old, repl, 1)
manifest.write_text(text)

sstable = Path("crates/db-storage-lsm/src/sstable.rs")
text = sstable.read_text()
for _ in range(2):
    if old not in text:
        raise SystemExit("missing SSTable UnsupportedVersion site")
    text = text.replace(
        old,
        '''return Err(DbError::UnsupportedVersion {\n            format: "LSM SSTable",\n            found: u64::from(version),\n            supported: u64::from(FORMAT_VERSION),\n        });''',
        1,
    )
sstable.write_text(text)

lib = Path("crates/db-storage-lsm/src/lib.rs")
text = lib.read_text()
text = text.replace(
'''struct Layout {\n    wal_path: PathBuf,\n    max_table_id: u64,\n    max_manifest_id: u64,\n}''',
'''struct Layout {\n    wal_path: PathBuf,\n    max_table_id: u64,\n    max_manifest_id: u64,\n    has_version_set: bool,\n}''')

old_open = '''        let layout = validate_layout(&path)?;\n        let version = manifest::load(&path)?;\n        let mut tables = Vec::with_capacity(version.tables.len());\n        for descriptor in &version.tables {\n            tables.push(SsTable::open(\n                &path.join(sstable_file_name(descriptor.table_id)),\n                descriptor.clone(),\n            )?);\n        }'''
new_open = '''        let layout = validate_layout(&path)?;\n        let version = if layout.has_version_set {\n            manifest::load(&path)?\n        } else {\n            VersionSet {\n                current_generation: 0,\n                manifest_id: 0,\n                durable_sequence: 0,\n                tables: Vec::new(),\n            }\n        };\n        let mut tables = Vec::with_capacity(version.tables.len());\n        for descriptor in &version.tables {\n            tables.push(SsTable::open(\n                &path.join(sstable_file_name(descriptor.table_id)),\n                descriptor.clone(),\n            )?);\n        }'''
if old_open not in text:
    raise SystemExit("missing open version-set block")
text = text.replace(old_open, new_open, 1)

old_verify = '''        let layout = validate_layout(path)?;\n        let version = manifest::load(path)?;\n        let mut sstable_entries = 0_u64;'''
new_verify = '''        let layout = validate_layout(path)?;\n        let version = if layout.has_version_set {\n            manifest::load(path)?\n        } else {\n            VersionSet {\n                current_generation: 0,\n                manifest_id: 0,\n                durable_sequence: 0,\n                tables: Vec::new(),\n            }\n        };\n        let mut sstable_entries = 0_u64;'''
if old_verify not in text:
    raise SystemExit("missing verify version-set block")
text = text.replace(old_verify, new_verify, 1)

old_flush = '''        while let Some((entries, durable_sequence)) = self.memtables.oldest_immutable_snapshot()? {\n            let table_id = self.next_table_id;'''
new_flush = '''        while let Some((entries, durable_sequence)) = self.memtables.oldest_immutable_snapshot()? {\n            if self.version.manifest_id == 0 {\n                self.version = manifest::create_initial(&self.path)?;\n                self.next_manifest_id = 2;\n            }\n            let table_id = self.next_table_id;'''
if old_flush not in text:
    raise SystemExit("missing flush loop")
text = text.replace(old_flush, new_flush, 1)

old_layout_tail = '''    if !found_current {\n        return Err(corruption(format!(\n            "LSM directory is missing required {CURRENT_FILE_NAME}"\n        )));\n    }\n    if max_manifest_id == 0 {\n        return Err(corruption("LSM directory contains no manifest snapshot"));\n    }\n\n    Ok(Layout {\n        wal_path: path.join(WAL_FILE_NAME),\n        max_table_id,\n        max_manifest_id,\n    })'''
new_layout_tail = '''    let has_version_set = match (found_current, max_manifest_id != 0) {\n        (true, true) => true,\n        (false, false) if max_table_id == 0 => false,\n        (false, false) => {\n            return Err(corruption(\n                "WAL-only legacy layout cannot contain SSTable files",\n            ));\n        }\n        (false, true) => {\n            return Err(corruption(\n                "LSM directory has manifest snapshots but is missing CURRENT",\n            ));\n        }\n        (true, false) => {\n            return Err(corruption(\n                "LSM directory has CURRENT but no manifest snapshot",\n            ));\n        }\n    };\n\n    Ok(Layout {\n        wal_path: path.join(WAL_FILE_NAME),\n        max_table_id,\n        max_manifest_id,\n        has_version_set,\n    })'''
if old_layout_tail not in text:
    raise SystemExit("missing validate_layout tail")
text = text.replace(old_layout_tail, new_layout_tail, 1)
lib.write_text(text)

# Existing frozen-MemTable regression now expects synchronous flush publication.
tests = Path("crates/db-storage-lsm/src/tests.rs")
text = tests.read_text()
text = text.replace(
    'assert_eq!(engine.stats().expect("stats").immutable_memtables, 1);',
    '''let first_flush = engine.stats().expect("stats after first flush");\n    assert_eq!(first_flush.immutable_memtables, 0);\n    assert_eq!(first_flush.sstables, 1);\n    assert!(first_flush.durable_sequence > 0);''',
    1,
)
text = text.replace(
    'assert!(before.immutable_memtables >= 2);',
    '''assert_eq!(before.immutable_memtables, 0);\n    assert!(before.sstables >= 2);\n    assert!(before.durable_sequence > 0);''',
    1,
)
tests.write_text(text)
