from pathlib import Path

path = Path(__file__).resolve().parents[2] / "crates/db-storage-lsm/src/instrumentation_tests.rs"
text = path.read_text()
text = text.replace("use db_core::KvEngine;", "use db_core::{KvEngine, ReadWorkUnit};")
text = text.replace("report.sorted_table_bytes_per_durable_live_byte,", "report.primary_structure_bytes_per_live_byte,")
text = text.replace("layered_report.point_read_tables_per_get,", "layered_report.point_read.ratio,")
text = text.replace("layered_report.range_versions_per_result,", "layered_report.range_read.ratio,")
text = text.replace(".sorted_table_bytes_per_durable_live_byte", ".primary_structure_bytes_per_live_byte")
needle = '''    assert_eq!(
        layered_report.point_read.ratio,
        AmplificationRatio {
            numerator: 5,
            denominator: 3,
        }
    );
'''
replacement = needle + '''    assert_eq!(layered_report.point_read.unit, ReadWorkUnit::LsmSstableConsult);
'''
if needle not in text:
    raise SystemExit("point-read migrated assertion marker missing")
text = text.replace(needle, replacement, 1)
needle = '''    assert_eq!(
        layered_report.range_read.ratio,
        AmplificationRatio {
            numerator: 10,
            denominator: 9,
        }
    );
'''
replacement = needle + '''    assert_eq!(
        layered_report.range_read.unit,
        ReadWorkUnit::LsmSstableVersionDecoded
    );
'''
if needle not in text:
    raise SystemExit("range-read migrated assertion marker missing")
text = text.replace(needle, replacement, 1)
path.write_text(text)
print("updated LSM amplification tests for common report schema")
