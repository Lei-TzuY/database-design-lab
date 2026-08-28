from pathlib import Path

# Remove the obsolete fixed-name constant now that every segment has a canonical computed name.
wal = Path("crates/db-storage-lsm/src/wal.rs")
text = wal.read_text()
old = 'pub(super) const WAL_FILE_NAME: &str = "wal-0000000000000001.log";\n'
if old not in text:
    raise SystemExit("missing obsolete WAL_FILE_NAME constant")
wal.write_text(text.replace(old, "", 1))

# The integration patch should import only the generic WAL naming/identity helpers.
lib = Path("crates/db-storage-lsm/src/lib.rs")
text = lib.read_text()
text = text.replace(
    '''use wal::{\n    file_name as wal_file_name, MutationKind, Wal, INITIAL_FIRST_SEQUENCE, INITIAL_WAL_ID,\n    WAL_FILE_NAME,\n};''',
    '''use wal::{\n    file_name as wal_file_name, MutationKind, Wal, INITIAL_FIRST_SEQUENCE, INITIAL_WAL_ID,\n};''',
    1,
)
lib.write_text(text)

# Keep the old WAL byte-level regressions, but target the canonical initial segment through the
# generic naming helper instead of a fixed production constant.
tests = Path("crates/db-storage-lsm/src/tests.rs")
text = tests.read_text()
old_import = '''use super::wal::{\n    checked_record_end, encode_record, MutationKind, RECORD_HEADER_LEN, WAL_FILE_NAME,\n    WAL_HEADER_LEN,\n};'''
new_import = '''use super::wal::{\n    checked_record_end, encode_record, file_name as wal_file_name, MutationKind, INITIAL_WAL_ID,\n    RECORD_HEADER_LEN, WAL_HEADER_LEN,\n};'''
if old_import not in text:
    raise SystemExit("missing WAL test import block")
text = text.replace(old_import, new_import, 1)
text = text.replace("join(WAL_FILE_NAME)", "join(wal_file_name(INITIAL_WAL_ID))")
tests.write_text(text)
