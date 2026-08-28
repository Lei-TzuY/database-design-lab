from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        raise SystemExit(f"missing {label}: {old[:160]!r}")
    return text.replace(old, new, 1)

# --- SSTable descriptor level metadata and level-aware creation ---
p = Path("crates/db-storage-lsm/src/sstable.rs")
text = p.read_text()
text = replace_once(
    text,
    "pub(super) struct SstableDescriptor {\n    pub(super) table_id: u64,\n",
    "pub(super) struct SstableDescriptor {\n    pub(super) table_id: u64,\n    pub(super) level: u32,\n",
    "descriptor level",
)
text = replace_once(
    text,
    "        Self::create_new_with_format(\n            directory,\n            table_id,\n            durable_sequence,\n            entries,\n            FORMAT_VERSION,\n        )\n    }\n\n    #[cfg(test)]",
    "        Self::create_new_with_format(\n            directory,\n            table_id,\n            0,\n            durable_sequence,\n            entries,\n            FORMAT_VERSION,\n        )\n    }\n\n    pub(super) fn create_new_at_level(\n        directory: &Path,\n        table_id: u64,\n        level: u32,\n        durable_sequence: u64,\n        entries: &BTreeMap<Vec<u8>, VersionedEntry>,\n    ) -> Result<Self> {\n        Self::create_new_with_format(\n            directory,\n            table_id,\n            level,\n            durable_sequence,\n            entries,\n            FORMAT_VERSION,\n        )\n    }\n\n    #[cfg(test)]",
    "level-aware create",
)
text = replace_once(
    text,
    "        Self::create_new_with_format(\n            directory,\n            table_id,\n            durable_sequence,\n            entries,\n            LEGACY_FORMAT_VERSION,\n        )",
    "        Self::create_new_with_format(\n            directory,\n            table_id,\n            0,\n            durable_sequence,\n            entries,\n            LEGACY_FORMAT_VERSION,\n        )",
    "legacy level zero",
)
text = replace_once(
    text,
    "        table_id: u64,\n        durable_sequence: u64,\n        entries: &BTreeMap<Vec<u8>, VersionedEntry>,\n        format_version: u16,",
    "        table_id: u64,\n        level: u32,\n        durable_sequence: u64,\n        entries: &BTreeMap<Vec<u8>, VersionedEntry>,\n        format_version: u16,",
    "create format signature",
)
text = replace_once(
    text,
    "        let descriptor = SstableDescriptor {\n            table_id,\n            file_bytes:",
    "        let descriptor = SstableDescriptor {\n            table_id,\n            level,\n            file_bytes:",
    "descriptor constructor level",
)
p.write_text(text)

# --- Manifest v3: levels become authoritative while v1/v2 descriptors remain readable as L0 ---
p = Path("crates/db-storage-lsm/src/manifest.rs")
text = p.read_text()
text = replace_once(
    text,
    "const MANIFEST_FORMAT_VERSION_V1: u16 = 1;\nconst MANIFEST_FORMAT_VERSION: u16 = 2;\nconst MANIFEST_HEADER_LEN_V1: usize = 64;\nconst MANIFEST_HEADER_LEN: usize = 80;\nconst DESCRIPTOR_PREFIX_LEN: usize = 40;",
    "const MANIFEST_FORMAT_VERSION_V1: u16 = 1;\nconst MANIFEST_FORMAT_VERSION_V2: u16 = 2;\nconst MANIFEST_FORMAT_VERSION: u16 = 3;\nconst MANIFEST_HEADER_LEN_V1: usize = 64;\nconst MANIFEST_HEADER_LEN: usize = 80;\nconst LEGACY_DESCRIPTOR_PREFIX_LEN: usize = 40;\nconst DESCRIPTOR_PREFIX_LEN: usize = 48;",
    "manifest v3 constants",
)
text = replace_once(
    text,
    "        MANIFEST_FORMAT_VERSION => {\n",
    "        MANIFEST_FORMAT_VERSION_V2 | MANIFEST_FORMAT_VERSION => {\n",
    "manifest v2/v3 header parser",
)
text = text.replace("invalid v2 manifest header fields", "invalid v2/v3 manifest header fields")
text = replace_once(
    text,
    "        let (descriptor, next) = decode_descriptor(&bytes, offset, body_end)?;",
    "        let (descriptor, next) =\n            decode_descriptor(&bytes, offset, body_end, format_version)?;",
    "descriptor version dispatch",
)
start = text.index("fn encode_descriptor(")
end = text.index("fn validate_version_set(", start)
new_descriptor_code = r'''fn encode_descriptor(descriptor: &SstableDescriptor) -> Result<Vec<u8>> {
    if descriptor.level > 1 {
        return Err(corruption(0, "manifest SSTable level exceeds implemented L1 policy"));
    }
    if descriptor.smallest_key.len() > MAX_KEY_BYTES || descriptor.largest_key.len() > MAX_KEY_BYTES
    {
        return Err(corruption(
            0,
            "manifest SSTable key bound exceeds common key limit",
        ));
    }
    let smallest_len = u32::try_from(descriptor.smallest_key.len())
        .map_err(|_| corruption(0, "manifest smallest-key length does not fit u32"))?;
    let largest_len = u32::try_from(descriptor.largest_key.len())
        .map_err(|_| corruption(0, "manifest largest-key length does not fit u32"))?;
    let capacity = DESCRIPTOR_PREFIX_LEN
        .checked_add(descriptor.smallest_key.len())
        .and_then(|len| len.checked_add(descriptor.largest_key.len()))
        .and_then(|len| len.checked_add(4))
        .ok_or_else(|| corruption(0, "manifest descriptor size overflowed usize"))?;
    let mut bytes = Vec::with_capacity(capacity);
    bytes.extend_from_slice(&descriptor.table_id.to_le_bytes());
    bytes.extend_from_slice(&descriptor.file_bytes.to_le_bytes());
    bytes.extend_from_slice(&descriptor.entry_count.to_le_bytes());
    bytes.extend_from_slice(&descriptor.durable_sequence.to_le_bytes());
    bytes.extend_from_slice(&descriptor.level.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&smallest_len.to_le_bytes());
    bytes.extend_from_slice(&largest_len.to_le_bytes());
    bytes.extend_from_slice(&descriptor.smallest_key);
    bytes.extend_from_slice(&descriptor.largest_key);
    let crc = crc32fast::hash(&bytes);
    bytes.extend_from_slice(&crc.to_le_bytes());
    Ok(bytes)
}

fn decode_descriptor(
    bytes: &[u8],
    offset: usize,
    limit: usize,
    manifest_format_version: u16,
) -> Result<(SstableDescriptor, usize)> {
    let legacy = manifest_format_version <= MANIFEST_FORMAT_VERSION_V2;
    let prefix_len = if legacy {
        LEGACY_DESCRIPTOR_PREFIX_LEN
    } else {
        DESCRIPTOR_PREFIX_LEN
    };
    let prefix_end = offset
        .checked_add(prefix_len)
        .ok_or_else(|| corruption(offset as u64, "manifest descriptor extent overflowed"))?;
    if prefix_end > limit {
        return Err(corruption(
            offset as u64,
            "truncated manifest descriptor prefix",
        ));
    }
    let prefix = &bytes[offset..prefix_end];
    let (level, smallest_len_offset, largest_len_offset) = if legacy {
        (0_u32, 32_usize, 36_usize)
    } else {
        if u32::from_le_bytes(prefix[36..40].try_into().expect("fixed slice")) != 0 {
            return Err(corruption(
                offset as u64 + 36,
                "manifest descriptor reserved field is nonzero",
            ));
        }
        (
            u32::from_le_bytes(prefix[32..36].try_into().expect("fixed slice")),
            40_usize,
            44_usize,
        )
    };
    let smallest_len = usize::try_from(u32::from_le_bytes(
        prefix[smallest_len_offset..smallest_len_offset + 4]
            .try_into()
            .expect("fixed slice"),
    ))
    .map_err(|_| {
        corruption(
            offset as u64 + smallest_len_offset as u64,
            "smallest-key length does not fit usize",
        )
    })?;
    let largest_len = usize::try_from(u32::from_le_bytes(
        prefix[largest_len_offset..largest_len_offset + 4]
            .try_into()
            .expect("fixed slice"),
    ))
    .map_err(|_| {
        corruption(
            offset as u64 + largest_len_offset as u64,
            "largest-key length does not fit usize",
        )
    })?;
    if level > 1 {
        return Err(corruption(
            offset as u64 + 32,
            "manifest SSTable level exceeds implemented L1 policy",
        ));
    }
    if smallest_len > MAX_KEY_BYTES || largest_len > MAX_KEY_BYTES {
        return Err(corruption(
            offset as u64 + smallest_len_offset as u64,
            "manifest key bound exceeds common key limit",
        ));
    }
    let smallest_end = prefix_end
        .checked_add(smallest_len)
        .ok_or_else(|| corruption(offset as u64, "manifest smallest-key extent overflowed"))?;
    let largest_end = smallest_end
        .checked_add(largest_len)
        .ok_or_else(|| corruption(offset as u64, "manifest largest-key extent overflowed"))?;
    let descriptor_end = largest_end.checked_add(4).ok_or_else(|| {
        corruption(
            offset as u64,
            "manifest descriptor checksum extent overflowed",
        )
    })?;
    if descriptor_end > limit {
        return Err(corruption(offset as u64, "truncated manifest descriptor"));
    }
    let expected_crc = u32::from_le_bytes(
        bytes[largest_end..descriptor_end]
            .try_into()
            .expect("fixed slice"),
    );
    if crc32fast::hash(&bytes[offset..largest_end]) != expected_crc {
        return Err(corruption(
            largest_end as u64,
            "manifest descriptor checksum mismatch",
        ));
    }
    let descriptor = SstableDescriptor {
        table_id: u64::from_le_bytes(prefix[0..8].try_into().expect("fixed slice")),
        level,
        file_bytes: u64::from_le_bytes(prefix[8..16].try_into().expect("fixed slice")),
        entry_count: u64::from_le_bytes(prefix[16..24].try_into().expect("fixed slice")),
        durable_sequence: u64::from_le_bytes(prefix[24..32].try_into().expect("fixed slice")),
        smallest_key: bytes[prefix_end..smallest_end].to_vec(),
        largest_key: bytes[smallest_end..largest_end].to_vec(),
    };
    if descriptor.table_id == 0
        || descriptor.file_bytes == 0
        || descriptor.entry_count == 0
        || descriptor.durable_sequence == 0
        || descriptor.smallest_key > descriptor.largest_key
    {
        return Err(corruption(
            offset as u64,
            "invalid manifest SSTable descriptor values",
        ));
    }
    Ok((descriptor, descriptor_end))
}

'''
text = text[:start] + new_descriptor_code + text[end:]
text = replace_once(
    text,
    "    let mut previous_table_id = 0_u64;\n    let mut previous_durable = 0_u64;\n    for descriptor in tables {",
    "    let mut previous_table_id = 0_u64;\n    let mut previous_durable = 0_u64;\n    let mut level1_tables = 0_usize;\n    for descriptor in tables {\n        if descriptor.level > 1 {\n            return Err(corruption(\n                0,\n                \"manifest SSTable level exceeds implemented L1 policy\",\n            ));\n        }\n        if descriptor.level == 1 {\n            level1_tables = level1_tables\n                .checked_add(1)\n                .ok_or_else(|| corruption(0, \"manifest L1 table count overflowed usize\"))?;\n            if level1_tables > 1 {\n                return Err(corruption(\n                    0,\n                    \"current L1 policy permits exactly one non-overlapping run\",\n                ));\n            }\n        }",
    "level validation",
)
p.write_text(text)

# --- Engine: structural level stats, deterministic L0 trigger, crash-published full-set compaction ---
p = Path("crates/db-storage-lsm/src/lib.rs")
text = p.read_text()
text = text.replace(
    "//! embeds a checksummed Bloom filter for point-read rejection; levels and compaction remain later\n//! Phase 3 work.",
    "//! embeds a checksummed Bloom filter for point-read rejection. Flushes enter overlapping L0; four\n//! L0 tables trigger a synchronous full-set merge into one non-overlapping L1 run, published through\n//! mirrored CURRENT before obsolete sorted-table/manifest files are reclaimed.",
)
text = replace_once(
    text,
    "pub const MUTABLE_MEMTABLE_BYTES_LIMIT: usize = 64 * 1024;",
    "pub const MUTABLE_MEMTABLE_BYTES_LIMIT: usize = 64 * 1024;\n\nconst LEVEL0_COMPACTION_TRIGGER: usize = 4;",
    "compaction trigger",
)
text = replace_once(
    text,
    "    /// Immutable sorted tables referenced by the authoritative manifest.\n    pub sstables: usize;",
    "    /// Immutable sorted tables referenced by the authoritative manifest.\n    pub sstables: usize,\n    /// Overlapping flush tables in level zero.\n    pub level0_sstables: usize,\n    /// Non-overlapping level-one runs. The current policy permits at most one.\n    pub level1_sstables: usize,",
    "stats level fields",
)
text = replace_once(
    text,
    "                sstables: version.tables.len(),\n                sstable_entries,",
    "                sstables: version.tables.len(),\n                level0_sstables: version.tables.iter().filter(|table| table.level == 0).count(),\n                level1_sstables: version.tables.iter().filter(|table| table.level == 1).count(),\n                sstable_entries,",
    "verify level stats",
)
text = replace_once(
    text,
    "            sstables: self.tables.len(),\n            sstable_entries,",
    "            sstables: self.tables.len(),\n            level0_sstables: self\n                .version\n                .tables\n                .iter()\n                .filter(|table| table.level == 0)\n                .count(),\n            level1_sstables: self\n                .version\n                .tables\n                .iter()\n                .filter(|table| table.level == 1)\n                .count(),\n            sstable_entries,",
    "live level stats",
)
text = replace_once(
    text,
    "        self.maybe_rotate_wal()?;\n        Ok(())\n    }\n\n    fn maybe_rotate_wal",
    "        self.maybe_compact_level0()?;\n        self.maybe_rotate_wal()?;\n        Ok(())\n    }\n\n    fn maybe_compact_level0(&mut self) -> Result<()> {\n        let level0_count = self\n            .version\n            .tables\n            .iter()\n            .filter(|table| table.level == 0)\n            .count();\n        if level0_count < LEVEL0_COMPACTION_TRIGGER {\n            return Ok(());\n        }\n\n        let mut merged = BTreeMap::new();\n        for table in &self.tables {\n            table.overlay_range(b\"\", None, &mut merged)?;\n        }\n        if merged.is_empty() {\n            return Err(corruption(\"full-set compaction unexpectedly produced no entries\"));\n        }\n\n        let table_id = self.next_table_id;\n        let manifest_id = self.next_manifest_id;\n        let next_table_id = checked_next_id(table_id, \"SSTable\")?;\n        let next_manifest_id = checked_next_id(manifest_id, \"manifest\")?;\n        let durable_sequence = self.version.durable_sequence;\n        let table = SsTable::create_new_at_level(\n            &self.path,\n            table_id,\n            1,\n            durable_sequence,\n            &merged,\n        )?;\n        let compacted = manifest::install(\n            &self.path,\n            &self.version,\n            manifest_id,\n            durable_sequence,\n            vec![table.descriptor().clone()],\n            self.version.wal_id,\n            self.version.wal_first_sequence,\n        )?;\n        let mirrored = manifest::mirror_current(&self.path, &compacted)?;\n        let active_table_id = table.descriptor().table_id;\n        let active_manifest_id = mirrored.manifest_id;\n\n        let old_tables = std::mem::replace(&mut self.tables, vec![table]);\n        self.version = mirrored;\n        self.next_table_id = next_table_id;\n        self.next_manifest_id = next_manifest_id;\n        drop(old_tables);\n        self.reclaim_obsolete_sstables(active_table_id);\n        self.reclaim_obsolete_manifests(active_manifest_id);\n        Ok(())\n    }\n\n    fn reclaim_obsolete_sstables(&self, active_table_id: u64) {\n        let Ok(entries) = fs::read_dir(&self.path) else {\n            return;\n        };\n        for entry in entries.flatten() {\n            let name = entry.file_name();\n            let text = name.to_string_lossy();\n            let Some(table_id) = parse_numbered_name(&text, \"sst-\", \".sst\") else {\n                continue;\n            };\n            if table_id != active_table_id {\n                let _ = fs::remove_file(entry.path());\n            }\n        }\n    }\n\n    fn reclaim_obsolete_manifests(&self, active_manifest_id: u64) {\n        let Ok(entries) = fs::read_dir(&self.path) else {\n            return;\n        };\n        for entry in entries.flatten() {\n            let name = entry.file_name();\n            let text = name.to_string_lossy();\n            let Some(manifest_id) = parse_numbered_name(&text, \"MANIFEST-\", \"\") else {\n                continue;\n            };\n            if manifest_id != active_manifest_id {\n                let _ = fs::remove_file(entry.path());\n            }\n        }\n    }\n\n    fn maybe_rotate_wal",
    "compaction methods",
)
text = replace_once(
    text,
    "        self.reclaim_obsolete_wals(new_wal_id);\n        debug_assert_ne!(old_wal_id, new_wal_id);",
    "        self.reclaim_obsolete_wals(new_wal_id);\n        self.reclaim_obsolete_manifests(self.version.manifest_id);\n        debug_assert_ne!(old_wal_id, new_wal_id);",
    "manifest reclamation after WAL mirror",
)
text = text.replace('            name: "lsm-segmented-wal-v2",', '            name: "lsm-level1-compaction-v3",')
text = replace_once(
    text,
    "#[cfg(test)]\nmod wal_rotation_tests;",
    "#[cfg(test)]\nmod wal_rotation_tests;\n#[cfg(test)]\nmod compaction_tests;",
    "compaction test module",
)
p.write_text(text)
