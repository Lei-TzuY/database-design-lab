from pathlib import Path

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
