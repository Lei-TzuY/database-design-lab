from pathlib import Path

# Add the injected error constructor to the pager test support.
path = Path("crates/db-storage-btree/src/lib.rs")
text = path.read_text()
marker = '''#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FaultSpec {
    event_index: usize,
    mode: FaultMode,
}
'''
insert = marker + '''
#[cfg(test)]
fn injected_fault(kind: DurableWriteKind, mode: FaultMode) -> BtreeError {
    BtreeError::Io(io::Error::other(format!(
        "injected durable-write fault at {kind:?} with mode {mode:?}"
    )))
}
'''
if marker not in text:
    raise SystemExit("missing FaultSpec marker")
path.write_text(text.replace(marker, insert, 1))

# Keep the matrix helper Clippy-clean without suppressing too_many_arguments.
path = Path("crates/db-storage-btree/src/tree/fault.rs")
text = path.read_text()
old = '''fn assert_put_fault(
    baseline: &Path,
    directory: &TempDir,
    case: usize,
    event_index: usize,
    site: DurableWriteKind,
    mode: FaultMode,
    old_value: &[u8],
    new_value: &[u8],
) {
    let path = clone_db(baseline, directory, &format!("put-{case}.db"));
'''
new = '''struct PutFaultCase<'a> {
    case: usize,
    event_index: usize,
    site: DurableWriteKind,
    mode: FaultMode,
    old_value: &'a [u8],
    new_value: &'a [u8],
}

fn assert_put_fault(baseline: &Path, directory: &TempDir, fault: PutFaultCase<'_>) {
    let PutFaultCase {
        case,
        event_index,
        site,
        mode,
        old_value,
        new_value,
    } = fault;
    let path = clone_db(baseline, directory, &format!("put-{case}.db"));
'''
if old not in text:
    raise SystemExit("missing assert_put_fault signature")
text = text.replace(old, new, 1)

old = '''            assert_put_fault(
                &baseline,
                &directory,
                case,
                index,
                site,
                mode,
                SMALL_OLD,
                &new_value,
            );
'''
new = '''            assert_put_fault(
                &baseline,
                &directory,
                PutFaultCase {
                    case,
                    event_index: index,
                    site,
                    mode,
                    old_value: SMALL_OLD,
                    new_value: &new_value,
                },
            );
'''
if old not in text:
    raise SystemExit("missing append fault call")
text = text.replace(old, new, 1)

old = '''            assert_put_fault(
                &baseline,
                &directory,
                case,
                index,
                site,
                mode,
                &second,
                &third,
            );
'''
new = '''            assert_put_fault(
                &baseline,
                &directory,
                PutFaultCase {
                    case,
                    event_index: index,
                    site,
                    mode,
                    old_value: &second,
                    new_value: &third,
                },
            );
'''
if old not in text:
    raise SystemExit("missing recycled fault call")
path.write_text(text.replace(old, new, 1))
