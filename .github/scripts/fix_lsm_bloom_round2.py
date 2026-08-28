from pathlib import Path

path = Path("crates/db-storage-lsm/src/bloom.rs")
text = path.read_text()
old = '''    pub(super) const fn bit_count(&self) -> u64 {
        self.bit_count
    }

    pub(super) const fn key_count(&self) -> u64 {
        self.key_count
    }
'''
new = '''    #[cfg(test)]
    pub(super) const fn bit_count(&self) -> u64 {
        self.bit_count
    }

    #[cfg(test)]
    pub(super) const fn key_count(&self) -> u64 {
        self.key_count
    }
'''
if old in text:
    text = text.replace(old, new, 1)
elif new not in text:
    raise SystemExit("Bloom accessor block not found")
path.write_text(text)
