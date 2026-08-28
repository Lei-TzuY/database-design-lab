from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        raise SystemExit(f"missing marker: {label}")
    return text.replace(old, new, 1)


# --- manifest.rs ----------------------------------------------------------
p = Path("crates/db-storage-lsm/src/manifest.rs")
text = p.read_text()
text = replace_once(
    text,
    "const MANIFEST_FORMAT_VERSION_V3: u16 = 3;\nconst MANIFEST_FORMAT_VERSION: u16 = 4;\nconst MANIFEST_HEADER_LEN_V1: usize = 64;\nconst MANIFEST_HEADER_LEN: usize = 80;",
    "const MANIFEST_FORMAT_VERSION_V3: u16 = 3;\nconst MANIFEST_FORMAT_VERSION_V4: u16 = 4;\nconst MANIFEST_FORMAT_VERSION: u16 = 5;\nconst MANIFEST_HEADER_LEN_V1: usize = 64;\nconst MANIFEST_HEADER_LEN_V4: usize = 80;\nconst MANIFEST_HEADER_LEN: usize = 88;",
    "manifest version constants",
)
text = replace_once(
    text,
    "    pub(super) durable_sequence: u64,\n    pub(super) tombstone_gc_sequence: u64,\n    pub(super) wal_id: u64,",
    "    pub(super) durable_sequence: u64,\n    pub(super) tombstone_gc_sequence: u64,\n    pub(super) table_id_high_watermark: u64,\n    pub(super) wal_id: u64,",
    "VersionSet allocation watermark",
)
text = replace_once(
    text,
    "        durable_sequence: 0,\n        tombstone_gc_sequence: 0,\n        wal_id,",
    "        durable_sequence: 0,\n        tombstone_gc_sequence: 0,\n        table_id_high_watermark: 0,\n        wal_id,",
    "initial allocation watermark",
)
text = replace_once(
    text,
    "    let generation = current\n        .current_generation\n        .checked_add(1)\n        .ok_or_else(|| corruption(0, \"CURRENT generation exhausted\"))?;\n    validate_version_set(\n        durable_sequence,\n        tombstone_gc_sequence,\n        wal_id,\n        wal_first_sequence,\n        &tables,\n    )?;\n    let next = VersionSet {\n        current_generation: generation,\n        manifest_id: new_manifest_id,\n        durable_sequence,\n        tombstone_gc_sequence,\n        wal_id,",
    "    let generation = current\n        .current_generation\n        .checked_add(1)\n        .ok_or_else(|| corruption(0, \"CURRENT generation exhausted\"))?;\n    let table_id_high_watermark = tables\n        .iter()\n        .fold(current.table_id_high_watermark, |high, descriptor| {\n            high.max(descriptor.table_id)\n        });\n    validate_version_set(\n        durable_sequence,\n        tombstone_gc_sequence,\n        table_id_high_watermark,\n        wal_id,\n        wal_first_sequence,\n        &tables,\n    )?;\n    let next = VersionSet {\n        current_generation: generation,\n        manifest_id: new_manifest_id,\n        durable_sequence,\n        tombstone_gc_sequence,\n        table_id_high_watermark,\n        wal_id,",
    "prepare install watermark",
)
text = replace_once(
    text,
    "    validate_version_set(\n        version.durable_sequence,\n        version.tombstone_gc_sequence,\n        version.wal_id,\n        version.wal_first_sequence,\n        &version.tables,\n    )?;",
    "    validate_version_set(\n        version.durable_sequence,\n        version.tombstone_gc_sequence,\n        version.table_id_high_watermark,\n        version.wal_id,\n        version.wal_first_sequence,\n        &version.tables,\n    )?;",
    "write validation watermark",
)
text = replace_once(
    text,
    "    bytes[64..72].copy_from_slice(&version.tombstone_gc_sequence.to_le_bytes());\n    let header_crc = crc32fast::hash(&bytes[..76]);\n    bytes[76..80].copy_from_slice(&header_crc.to_le_bytes());",
    "    bytes[64..72].copy_from_slice(&version.tombstone_gc_sequence.to_le_bytes());\n    bytes[72..80].copy_from_slice(&version.table_id_high_watermark.to_le_bytes());\n    let header_crc = crc32fast::hash(&bytes[..84]);\n    bytes[84..88].copy_from_slice(&header_crc.to_le_bytes());",
    "v5 header encoding",
)
start = text.index("    let (header_len, wal_id, wal_first_sequence, tombstone_gc_sequence) = match format_version {")
end = text.index("\n\n    let encoded_manifest_id", start)
new_match = '''    let (\n        header_len,\n        wal_id,\n        wal_first_sequence,\n        tombstone_gc_sequence,\n        encoded_table_id_high_watermark,\n    ) = match format_version {\n        MANIFEST_FORMAT_VERSION_V1 => {\n            if u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) as usize\n                != MANIFEST_HEADER_LEN_V1\n                || bytes[12..16] != [0; 4]\n                || bytes[48..60].iter().any(|byte| *byte != 0)\n            {\n                return Err(corruption(10, "invalid v1 manifest header fields"));\n            }\n            let expected_header_crc =\n                u32::from_le_bytes(bytes[60..64].try_into().expect("fixed slice"));\n            if crc32fast::hash(&bytes[..60]) != expected_header_crc {\n                return Err(corruption(60, "manifest header checksum mismatch"));\n            }\n            (MANIFEST_HEADER_LEN_V1, 1, 1, 0, None)\n        }\n        MANIFEST_FORMAT_VERSION_V2 | MANIFEST_FORMAT_VERSION_V3 => {\n            if bytes.len() < MANIFEST_HEADER_LEN_V4 + 4\n                || u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) as usize\n                    != MANIFEST_HEADER_LEN_V4\n                || bytes[12..16] != [0; 4]\n                || bytes[64..76].iter().any(|byte| *byte != 0)\n            {\n                return Err(corruption(10, "invalid v2/v3 manifest header fields"));\n            }\n            let expected_header_crc =\n                u32::from_le_bytes(bytes[76..80].try_into().expect("fixed slice"));\n            if crc32fast::hash(&bytes[..76]) != expected_header_crc {\n                return Err(corruption(76, "manifest header checksum mismatch"));\n            }\n            (\n                MANIFEST_HEADER_LEN_V4,\n                u64::from_le_bytes(bytes[48..56].try_into().expect("fixed slice")),\n                u64::from_le_bytes(bytes[56..64].try_into().expect("fixed slice")),\n                0,\n                None,\n            )\n        }\n        MANIFEST_FORMAT_VERSION_V4 => {\n            if bytes.len() < MANIFEST_HEADER_LEN_V4 + 4\n                || u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) as usize\n                    != MANIFEST_HEADER_LEN_V4\n                || bytes[12..16] != [0; 4]\n                || bytes[72..76].iter().any(|byte| *byte != 0)\n            {\n                return Err(corruption(10, "invalid v4 manifest header fields"));\n            }\n            let expected_header_crc =\n                u32::from_le_bytes(bytes[76..80].try_into().expect("fixed slice"));\n            if crc32fast::hash(&bytes[..76]) != expected_header_crc {\n                return Err(corruption(76, "manifest header checksum mismatch"));\n            }\n            (\n                MANIFEST_HEADER_LEN_V4,\n                u64::from_le_bytes(bytes[48..56].try_into().expect("fixed slice")),\n                u64::from_le_bytes(bytes[56..64].try_into().expect("fixed slice")),\n                u64::from_le_bytes(bytes[64..72].try_into().expect("fixed slice")),\n                None,\n            )\n        }\n        MANIFEST_FORMAT_VERSION => {\n            if bytes.len() < MANIFEST_HEADER_LEN + 4\n                || u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) as usize\n                    != MANIFEST_HEADER_LEN\n                || bytes[12..16] != [0; 4]\n                || bytes[80..84].iter().any(|byte| *byte != 0)\n            {\n                return Err(corruption(10, "invalid v5 manifest header fields"));\n            }\n            let expected_header_crc =\n                u32::from_le_bytes(bytes[84..88].try_into().expect("fixed slice"));\n            if crc32fast::hash(&bytes[..84]) != expected_header_crc {\n                return Err(corruption(84, "manifest header checksum mismatch"));\n            }\n            (\n                MANIFEST_HEADER_LEN,\n                u64::from_le_bytes(bytes[48..56].try_into().expect("fixed slice")),\n                u64::from_le_bytes(bytes[56..64].try_into().expect("fixed slice")),\n                u64::from_le_bytes(bytes[64..72].try_into().expect("fixed slice")),\n                Some(u64::from_le_bytes(\n                    bytes[72..80].try_into().expect("fixed slice"),\n                )),\n            )\n        }\n        _ => {\n            return Err(DbError::UnsupportedVersion {\n                format: "LSM manifest",\n                found: u64::from(format_version),\n                supported: u64::from(MANIFEST_FORMAT_VERSION),\n            });\n        }\n    };'''
text = text[:start] + new_match + text[end:]
text = replace_once(
    text,
    "    validate_version_set(\n        durable_sequence,\n        tombstone_gc_sequence,\n        wal_id,\n        wal_first_sequence,\n        &tables,\n    )?;\n    Ok(VersionSet {\n        current_generation: 0,\n        manifest_id,\n        durable_sequence,\n        tombstone_gc_sequence,\n        wal_id,",
    "    let active_table_id_high_watermark = tables\n        .iter()\n        .map(|descriptor| descriptor.table_id)\n        .max()\n        .unwrap_or(0);\n    let table_id_high_watermark = encoded_table_id_high_watermark.unwrap_or_else(|| {\n        if tables.is_empty() && durable_sequence > 0 {\n            durable_sequence.max(active_table_id_high_watermark)\n        } else {\n            active_table_id_high_watermark\n        }\n    });\n    validate_version_set(\n        durable_sequence,\n        tombstone_gc_sequence,\n        table_id_high_watermark,\n        wal_id,\n        wal_first_sequence,\n        &tables,\n    )?;\n    Ok(VersionSet {\n        current_generation: 0,\n        manifest_id,\n        durable_sequence,\n        tombstone_gc_sequence,\n        table_id_high_watermark,\n        wal_id,",
    "read allocation watermark",
)
text = replace_once(
    text,
    "fn validate_version_set(\n    durable_sequence: u64,\n    tombstone_gc_sequence: u64,\n    wal_id: u64,",
    "fn validate_version_set(\n    durable_sequence: u64,\n    tombstone_gc_sequence: u64,\n    table_id_high_watermark: u64,\n    wal_id: u64,",
    "validation signature",
)
text = replace_once(
    text,
    "    if wal_id == 0 || wal_first_sequence == 0 {",
    "    if durable_sequence > 0 && table_id_high_watermark == 0 {\n        return Err(corruption(\n            72,\n            \"nonempty durable history requires a nonzero SSTable id high watermark\",\n        ));\n    }\n    if wal_id == 0 || wal_first_sequence == 0 {",
    "nonzero history watermark validation",
)
text = replace_once(
    text,
    "    let expected = tables.last().map_or(0, |table| table.durable_sequence);",
    "    if previous_table_id > table_id_high_watermark {\n        return Err(corruption(\n            72,\n            format!(\n                \"manifest SSTable id high watermark {table_id_high_watermark} is below active table id {previous_table_id}\"\n            ),\n        ));\n    }\n    let expected = tables.last().map_or(0, |table| table.durable_sequence);",
    "active table watermark validation",
)
p.write_text(text)


# --- lib.rs ---------------------------------------------------------------
p = Path("crates/db-storage-lsm/src/lib.rs")
text = p.read_text()
text = text.replace("records that proof point in Manifest v4", "records that proof point in Manifest v5")
text = text.replace('name: "lsm-level1-tombstone-gc-v4"', 'name: "lsm-level1-tombstone-gc-v5"')
text = text.replace(
    "                durable_sequence: 0,\n                tombstone_gc_sequence: 0,\n                wal_id: INITIAL_WAL_ID,",
    "                durable_sequence: 0,\n                tombstone_gc_sequence: 0,\n                table_id_high_watermark: 0,\n                wal_id: INITIAL_WAL_ID,",
)
old = '''        let version = if layout.has_version_set {\n            manifest::load(&path)?\n        } else {\n            VersionSet {'''
new = '''        let mut version = if layout.has_version_set {\n            manifest::load(&path)?\n        } else {\n            VersionSet {'''
text = replace_once(text, old, new, "mutable opened version")
marker = '''        };\n        let mut tables = Vec::with_capacity(version.tables.len());'''
replacement = '''        };\n        version.table_id_high_watermark = version\n            .table_id_high_watermark\n            .max(layout.max_table_id);\n        let mut tables = Vec::with_capacity(version.tables.len());'''
text = replace_once(text, marker, replacement, "observed layout watermark")
text = replace_once(
    text,
    '            next_table_id: checked_next_id(layout.max_table_id, "SSTable")?,',
    '            next_table_id: checked_next_id(version.table_id_high_watermark, "SSTable")?,',
    "next table allocation",
)
p.write_text(text)


# --- compaction_tests.rs --------------------------------------------------
p = Path("crates/db-storage-lsm/src/compaction_tests.rs")
text = p.read_text()
text = text.replace(
    "use super::{LsmEngine, MUTABLE_MEMTABLE_BYTES_LIMIT};",
    "use super::{\n    CompactionFaultMode, CompactionWriteKind, LsmEngine, MUTABLE_MEMTABLE_BYTES_LIMIT,\n};",
)
old = '''fn rewrite_manifest_checksums(bytes: &mut [u8]) {\n    let header_crc = crc32fast::hash(&bytes[..76]);\n    bytes[76..80].copy_from_slice(&header_crc.to_le_bytes());\n    let file_crc_offset = bytes.len() - 4;\n    let file_crc = crc32fast::hash(&bytes[..file_crc_offset]);\n    bytes[file_crc_offset..].copy_from_slice(&file_crc.to_le_bytes());\n}\n'''
new = '''fn rewrite_manifest_checksums(bytes: &mut [u8]) {\n    let header_len = usize::from(u16::from_le_bytes(\n        bytes[10..12].try_into().expect("manifest header length"),\n    ));\n    let header_crc_offset = header_len.checked_sub(4).expect("manifest header CRC offset");\n    let header_crc = crc32fast::hash(&bytes[..header_crc_offset]);\n    bytes[header_crc_offset..header_len].copy_from_slice(&header_crc.to_le_bytes());\n    let file_crc_offset = bytes.len() - 4;\n    let file_crc = crc32fast::hash(&bytes[..file_crc_offset]);\n    bytes[file_crc_offset..].copy_from_slice(&file_crc.to_le_bytes());\n}\n'''
text = replace_once(text, old, new, "generic manifest checksum helper")
start = text.index("fn rewrite_manifest_as_v3(path: &Path) {")
end = text.index("\n\nfn populate_four_flushes", start)
replacement = r'''fn rewrite_manifest_as_v3(path: &Path) {
    let manifest_path = only_manifest(path);
    let source = fs::read(&manifest_path).expect("read v5 manifest fixture");
    assert_eq!(
        u16::from_le_bytes(source[8..10].try_into().expect("manifest version")),
        5
    );
    let source_header_len = usize::from(u16::from_le_bytes(
        source[10..12].try_into().expect("source header length"),
    ));
    assert_eq!(source_header_len, 88);
    let source_file_crc = source.len() - 4;

    let mut legacy = vec![0_u8; 80];
    legacy[0..8].copy_from_slice(b"DBLSMMAN");
    legacy[8..10].copy_from_slice(&3_u16.to_le_bytes());
    legacy[10..12].copy_from_slice(&80_u16.to_le_bytes());
    legacy[16..64].copy_from_slice(&source[16..64]);
    let header_crc = crc32fast::hash(&legacy[..76]);
    legacy[76..80].copy_from_slice(&header_crc.to_le_bytes());
    legacy.extend_from_slice(&source[source_header_len..source_file_crc]);
    let file_crc = crc32fast::hash(&legacy);
    legacy.extend_from_slice(&file_crc.to_le_bytes());

    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(manifest_path)
        .expect("open manifest fixture for downgrade");
    file.write_all(&legacy).expect("write Manifest v3 fixture");
    file.sync_all().expect("sync Manifest v3 fixture");
}

fn rewrite_manifest_as_v4(path: &Path) {
    let manifest_path = only_manifest(path);
    let source = fs::read(&manifest_path).expect("read v5 manifest fixture");
    assert_eq!(
        u16::from_le_bytes(source[8..10].try_into().expect("manifest version")),
        5
    );
    let source_header_len = usize::from(u16::from_le_bytes(
        source[10..12].try_into().expect("source header length"),
    ));
    assert_eq!(source_header_len, 88);
    let source_file_crc = source.len() - 4;

    let mut legacy = vec![0_u8; 80];
    legacy[0..8].copy_from_slice(b"DBLSMMAN");
    legacy[8..10].copy_from_slice(&4_u16.to_le_bytes());
    legacy[10..12].copy_from_slice(&80_u16.to_le_bytes());
    legacy[16..72].copy_from_slice(&source[16..72]);
    let header_crc = crc32fast::hash(&legacy[..76]);
    legacy[76..80].copy_from_slice(&header_crc.to_le_bytes());
    legacy.extend_from_slice(&source[source_header_len..source_file_crc]);
    let file_crc = crc32fast::hash(&legacy);
    legacy.extend_from_slice(&file_crc.to_le_bytes());

    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(manifest_path)
        .expect("open manifest fixture for v4 downgrade");
    file.write_all(&legacy).expect("write Manifest v4 fixture");
    file.sync_all().expect("sync Manifest v4 fixture");
}'''
text = text[:start] + replacement + text[end:]
start = text.index("fn rewrite_single_table_manifest_as_v2(path: &Path, manifest_id: u64) {")
end = text.index("\n\n#[test]\nfn four_overlapping", start)
replacement = r'''fn rewrite_single_table_manifest_as_v2(path: &Path, manifest_id: u64) {
    let manifest_path = path.join(format!("MANIFEST-{manifest_id:016}"));
    let source = fs::read(&manifest_path).expect("read v5 manifest fixture");
    assert_eq!(
        u16::from_le_bytes(source[8..10].try_into().expect("manifest version")),
        5
    );
    assert_eq!(
        u64::from_le_bytes(source[32..40].try_into().expect("table count")),
        1
    );
    let base = usize::from(u16::from_le_bytes(
        source[10..12].try_into().expect("source header length"),
    ));
    assert_eq!(base, 88);
    assert_eq!(
        u32::from_le_bytes(source[base + 32..base + 36].try_into().expect("v5 level")),
        0,
        "only an L0 v5 descriptor can be represented by legacy Manifest v2"
    );
    assert_eq!(source[base + 36..base + 40], [0; 4]);

    let smallest_len = usize::try_from(u32::from_le_bytes(
        source[base + 40..base + 44]
            .try_into()
            .expect("smallest length"),
    ))
    .expect("smallest length fits usize");
    let largest_len = usize::try_from(u32::from_le_bytes(
        source[base + 44..base + 48]
            .try_into()
            .expect("largest length"),
    ))
    .expect("largest length fits usize");
    let keys_start = base + 48;
    let keys_end = keys_start + smallest_len + largest_len;
    assert!(keys_end + 8 <= source.len());

    let mut descriptor = Vec::new();
    descriptor.extend_from_slice(&source[base..base + 32]);
    descriptor.extend_from_slice(&source[base + 40..base + 48]);
    descriptor.extend_from_slice(&source[keys_start..keys_end]);
    let descriptor_crc = crc32fast::hash(&descriptor);
    descriptor.extend_from_slice(&descriptor_crc.to_le_bytes());

    let mut legacy = vec![0_u8; 80];
    legacy[0..8].copy_from_slice(b"DBLSMMAN");
    legacy[8..10].copy_from_slice(&2_u16.to_le_bytes());
    legacy[10..12].copy_from_slice(&80_u16.to_le_bytes());
    legacy[16..40].copy_from_slice(&source[16..40]);
    legacy[40..48].copy_from_slice(
        &u64::try_from(descriptor.len())
            .expect("descriptor length fits u64")
            .to_le_bytes(),
    );
    legacy[48..64].copy_from_slice(&source[48..64]);
    let header_crc = crc32fast::hash(&legacy[..76]);
    legacy[76..80].copy_from_slice(&header_crc.to_le_bytes());
    legacy.extend_from_slice(&descriptor);
    let file_crc = crc32fast::hash(&legacy);
    legacy.extend_from_slice(&file_crc.to_le_bytes());

    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&manifest_path)
        .expect("open manifest fixture for downgrade");
    file.write_all(&legacy).expect("write Manifest v2 fixture");
    file.sync_all().expect("sync Manifest v2 fixture");
}'''
text = text[:start] + replacement + text[end:]
# Latest-version terminology in existing regressions.
for old_s, new_s in [
    ("upgrades_through_v4_compaction", "upgrades_through_v5_compaction"),
    ("v4 source stats", "v5 source stats"),
    ("v4 upgraded stats", "v5 upgraded stats"),
    ("reopen upgraded v4 L1", "reopen upgraded v5 L1"),
    ("reopened v4 stats", "reopened v5 stats"),
    ("manifest_v3_reopens_and_upgrades_to_v4_on_next_install", "manifest_v3_reopens_and_upgrades_to_v5_on_next_install"),
    ("publish Manifest v4", "publish Manifest v5"),
    ("reopen upgraded Manifest v4", "reopen upgraded Manifest v5"),
    ("manifest_v4_rejects_unproven_gc_tableless_watermarks_and_reserved_bytes", "manifest_v5_rejects_unproven_gc_tableless_and_allocation_watermarks"),
]:
    text = text.replace(old_s, new_s)
# Latest manifest assertions.
text = text.replace(
    'u16::from_le_bytes(manifest[8..10].try_into().expect("manifest version")),\n        4',
    'u16::from_le_bytes(manifest[8..10].try_into().expect("manifest version")),\n        5',
)
text = text.replace(
    'u16::from_le_bytes(manifest[8..10].try_into().expect("manifest version")),\n        4\n    );\n    reopened.reopen().expect("reopen upgraded Manifest v5");',
    'u16::from_le_bytes(manifest[8..10].try_into().expect("manifest version")),\n        5\n    );\n    reopened.reopen().expect("reopen upgraded Manifest v5");',
)
# v5 table-less header carries both GC and allocation watermarks.
needle = '''    assert_eq!(\n        u64::from_le_bytes(manifest[64..72].try_into().expect("GC sequence")),\n        64\n    );'''
replacement = needle + '''\n    assert_eq!(\n        u64::from_le_bytes(\n            manifest[72..80]\n                .try_into()\n                .expect("SSTable id high watermark"),\n        ),\n        4\n    );'''
text = replace_once(text, needle, replacement, "tableless v5 allocation watermark assertion")
# v5 reserved bytes moved after the new watermark.
text = text.replace("    invalid_reserved[72] = 1;", "    invalid_reserved[80] = 1;")
text = text.replace("nonzero v4 reserved byte", "nonzero v5 reserved byte")
text = text.replace("invalid v4 manifest", "invalid v5 manifest")
# Add checksum-valid allocation watermark corruption checks into the semantic test.
needle = '''    let error = LsmEngine::open(&live_path).expect_err("nonzero v5 reserved byte must fail");\n    assert!(error.to_string().contains("invalid v5 manifest"), "{error}");'''
replacement = needle + '''\n\n    let mut invalid_high_watermark = original.clone();\n    invalid_high_watermark[72..80].copy_from_slice(&4_u64.to_le_bytes());\n    rewrite_manifest_checksums(&mut invalid_high_watermark);\n    fs::write(&live_manifest, invalid_high_watermark)\n        .expect("write low SSTable id high watermark");\n    let error = LsmEngine::open(&live_path)\n        .expect_err("allocation watermark below the active L1 id must fail");\n    assert!(error.to_string().contains("high watermark"), "{error}");'''
text = replace_once(text, needle, replacement, "active-id corruption regression")
needle = '''    let error = LsmEngine::open(&empty_path).expect_err("table-less watermark must be GC-covered");\n    assert!(error.to_string().contains("table-less manifest"), "{error}");'''
replacement = needle + '''\n\n    let mut invalid_allocation = fs::read(&empty_manifest).expect("read restored table-less manifest");\n    invalid_allocation[64..72].copy_from_slice(&64_u64.to_le_bytes());\n    invalid_allocation[72..80].copy_from_slice(&0_u64.to_le_bytes());\n    rewrite_manifest_checksums(&mut invalid_allocation);\n    fs::write(&empty_manifest, invalid_allocation)\n        .expect("write zero allocation watermark with durable history");\n    let error = LsmEngine::open(&empty_path)\n        .expect_err("durable history without an allocation watermark must fail");\n    assert!(error.to_string().contains("high watermark"), "{error}");'''
text = replace_once(text, needle, replacement, "tableless allocation corruption regression")
# Add explicit v4 reader/migration evidence and the orphan-floor regression.
insert_at = text.index("\n\n#[test]\nfn manifest_v5_rejects_unproven_gc_tableless_and_allocation_watermarks")
extra = r'''

#[test]
fn manifest_v4_reopens_and_upgrades_to_v5_on_next_install() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    {
        let mut engine = LsmEngine::create_new(&path).expect("create LSM engine");
        engine.put(b"legacy-a", &large_value(0xb1)).expect("put a");
        engine
            .put(b"legacy-b", &large_value(0xb2))
            .expect("flush first L0");
    }
    rewrite_manifest_as_v4(&path);

    let mut reopened = LsmEngine::open(&path).expect("open legacy Manifest v4");
    reopened
        .put(b"new-a", &large_value(0xb3))
        .expect("put new a");
    reopened
        .put(b"new-b", &large_value(0xb4))
        .expect("publish Manifest v5");
    let manifest = fs::read(only_manifest(&path)).expect("read upgraded manifest");
    assert_eq!(
        u16::from_le_bytes(manifest[8..10].try_into().expect("manifest version")),
        5
    );
    assert!(
        u64::from_le_bytes(
            manifest[72..80]
                .try_into()
                .expect("SSTable id high watermark"),
        ) >= 2
    );
    reopened.reopen().expect("reopen upgraded Manifest v5");
    assert_eq!(
        reopened.get(b"legacy-a").expect("get legacy key"),
        Some(large_value(0xb1))
    );
}

#[test]
fn tableless_gc_persists_observed_orphan_id_floor_before_cleanup() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    let mut engine = LsmEngine::create_new(&path).expect("create LSM engine");

    for batch in 0_u8..3 {
        for index in 0_u8..16 {
            let mut key = vec![0_u8; MAX_KEY_BYTES];
            key[0] = batch;
            key[1] = index;
            assert_eq!(engine.delete(&key).expect("build three L0 tables"), None);
        }
    }
    assert_eq!(engine.stats().expect("three-L0 stats").level0_sstables, 3);

    engine.inject_compaction_fault_for_test(
        CompactionWriteKind::Manifest,
        CompactionFaultMode::BeforeWrite,
    );
    for index in 0_u8..15 {
        let mut key = vec![0_u8; MAX_KEY_BYTES];
        key[0] = 3;
        key[1] = index;
        assert_eq!(engine.delete(&key).expect("pre-trigger tombstone"), None);
    }
    let mut trigger = vec![0_u8; MAX_KEY_BYTES];
    trigger[0] = 3;
    trigger[1] = 15;
    assert!(engine.delete(&trigger).is_err(), "compaction fault must escape");
    drop(engine);

    assert_eq!(canonical_count(&path, "sst-", ".sst"), 4);
    let orphan = path.join("sst-0000000000000099.sst");
    fs::write(&orphan, b"ambiguous canonical crash orphan")
        .expect("create canonical orphan id 99");

    let mut reopened = LsmEngine::open(&path).expect("open four L0 tables plus orphan 99");
    reopened
        .put(b"tail", b"v")
        .expect("retry table-less compaction without another SSTable allocation");
    let checkpoint = reopened.stats().expect("table-less checkpoint stats");
    assert_eq!(checkpoint.sstables, 0);
    assert_eq!(checkpoint.durable_sequence, 64);
    assert_eq!(checkpoint.tombstone_gc_sequence, 64);
    assert_eq!(checkpoint.mutable_entries, 1);
    assert_eq!(canonical_count(&path, "sst-", ".sst"), 0);
    assert!(!orphan.exists(), "orphan cleanup occurs only after v5 publication");

    let manifest = fs::read(only_manifest(&path)).expect("read v5 table-less checkpoint");
    assert_eq!(
        u64::from_le_bytes(
            manifest[72..80]
                .try_into()
                .expect("SSTable id high watermark"),
        ),
        99
    );

    reopened.reopen().expect("reopen after orphan cleanup");
    reopened
        .put(b"fill-a", &large_value(0xc1))
        .expect("put first filler");
    reopened
        .put(b"fill-b", &large_value(0xc2))
        .expect("flush first post-checkpoint L0");
    assert!(
        path.join("sst-0000000000000100.sst").exists(),
        "allocation must continue above the cleaned-up ambiguous orphan id"
    );
    reopened.reopen().expect("reopen table 100");
    assert_eq!(
        reopened.get(b"tail").expect("read WAL-tail value"),
        Some(b"v".to_vec())
    );
}
'''
text = text[:insert_at] + extra + text[insert_at:]
p.write_text(text)
