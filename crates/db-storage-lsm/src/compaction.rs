use db_core::{DbError, Result};

use crate::sstable::SstableDescriptor;

pub(super) const L0_COMPACTION_TRIGGER: usize = 4;
pub(super) const MAX_LEVEL: u32 = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CompactionPlan {
    pub(super) source_level: u32,
    pub(super) target_level: u32,
    pub(super) source_table_ids: Vec<u64>,
    pub(super) target_overlap_ids: Vec<u64>,
    pub(super) smallest_key: Vec<u8>,
    pub(super) largest_key: Vec<u8>,
}

pub(super) fn plan_l0_to_l1(tables: &[SstableDescriptor]) -> Result<Option<CompactionPlan>> {
    validate_level_invariants(tables)?;
    let mut l0 = tables
        .iter()
        .filter(|table| table.level == 0)
        .collect::<Vec<_>>();
    if l0.len() < L0_COMPACTION_TRIGGER {
        return Ok(None);
    }
    l0.sort_by_key(|table| table.table_id);
    let selected = &l0[..L0_COMPACTION_TRIGGER];
    let mut smallest_key = selected[0].smallest_key.clone();
    let mut largest_key = selected[0].largest_key.clone();
    let mut source_table_ids = Vec::with_capacity(L0_COMPACTION_TRIGGER);
    for table in selected {
        source_table_ids.push(table.table_id);
        if table.smallest_key < smallest_key {
            smallest_key = table.smallest_key.clone();
        }
        if table.largest_key > largest_key {
            largest_key = table.largest_key.clone();
        }
    }

    let mut target_overlap_ids = tables
        .iter()
        .filter(|table| {
            table.level == 1
                && ranges_overlap(
                    &smallest_key,
                    &largest_key,
                    &table.smallest_key,
                    &table.largest_key,
                )
        })
        .map(|table| table.table_id)
        .collect::<Vec<_>>();
    target_overlap_ids.sort_unstable();

    Ok(Some(CompactionPlan {
        source_level: 0,
        target_level: 1,
        source_table_ids,
        target_overlap_ids,
        smallest_key,
        largest_key,
    }))
}

pub(super) fn validate_level_invariants(tables: &[SstableDescriptor]) -> Result<()> {
    for table in tables {
        if table.level > MAX_LEVEL {
            return Err(corruption(format!(
                "SSTable {} declares unsupported level {} above {MAX_LEVEL}",
                table.table_id, table.level
            )));
        }
    }

    for level in 1..=MAX_LEVEL {
        let mut current = tables
            .iter()
            .filter(|table| table.level == level)
            .collect::<Vec<_>>();
        current.sort_by(|left, right| {
            left.smallest_key
                .cmp(&right.smallest_key)
                .then(left.table_id.cmp(&right.table_id))
        });
        for pair in current.windows(2) {
            if pair[0].largest_key >= pair[1].smallest_key {
                return Err(corruption(format!(
                    "level {level} SSTables {} and {} overlap or touch out of canonical order",
                    pair[0].table_id, pair[1].table_id
                )));
            }
        }
    }
    Ok(())
}

fn ranges_overlap(
    left_smallest: &[u8],
    left_largest: &[u8],
    right_smallest: &[u8],
    right_largest: &[u8],
) -> bool {
    left_smallest <= right_largest && right_smallest <= left_largest
}

fn corruption(reason: impl Into<String>) -> DbError {
    DbError::Corruption {
        offset: 0,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(id: u64, level: u32, smallest: &[u8], largest: &[u8]) -> SstableDescriptor {
        SstableDescriptor {
            table_id: id,
            file_bytes: 100,
            entry_count: 1,
            durable_sequence: id,
            level,
            smallest_key: smallest.to_vec(),
            largest_key: largest.to_vec(),
        }
    }

    #[test]
    fn level_zero_may_overlap_but_nonzero_levels_must_not() {
        let l0 = vec![
            table(1, 0, b"a", b"z"),
            table(2, 0, b"b", b"y"),
        ];
        validate_level_invariants(&l0).expect("L0 overlap is legal");

        let l1 = vec![table(1, 1, b"a", b"m"), table(2, 1, b"n", b"z")];
        validate_level_invariants(&l1).expect("disjoint L1 ranges are legal");

        let overlap = vec![table(1, 1, b"a", b"m"), table(2, 1, b"m", b"z")];
        assert!(validate_level_invariants(&overlap).is_err());
        assert!(validate_level_invariants(&[table(9, MAX_LEVEL + 1, b"a", b"b")]).is_err());
    }

    #[test]
    fn planner_waits_for_four_l0_tables_then_selects_oldest_and_all_l1_overlaps() {
        let mut tables = vec![
            table(1, 0, b"m", b"z"),
            table(2, 0, b"a", b"c"),
            table(3, 0, b"f", b"h"),
        ];
        assert_eq!(plan_l0_to_l1(&tables).expect("plan"), None);

        tables.extend([
            table(4, 0, b"d", b"p"),
            table(5, 0, b"zz", b"zzz"),
            table(6, 1, b"0", b"0z"),
            table(7, 1, b"b", b"e"),
            table(8, 1, b"i", b"k"),
            table(9, 1, b"q", b"zz"),
        ]);
        let plan = plan_l0_to_l1(&tables)
            .expect("plan")
            .expect("triggered plan");
        assert_eq!(plan.source_level, 0);
        assert_eq!(plan.target_level, 1);
        assert_eq!(plan.source_table_ids, vec![1, 2, 3, 4]);
        assert_eq!(plan.target_overlap_ids, vec![7, 8, 9]);
        assert_eq!(plan.smallest_key, b"a");
        assert_eq!(plan.largest_key, b"z");
    }
}
