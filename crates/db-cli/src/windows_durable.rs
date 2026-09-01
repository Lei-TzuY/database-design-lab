//! Audited Win32 namespace-durability primitives.
//!
//! This module is the only `db-cli` source module allowed to contain raw FFI. The surrounding
//! package keeps `unsafe_code = "deny"`, and `lib.rs` grants a local allowance only to this module.
//! The first primitive deliberately exposes one narrow operation whose Win32 durability semantics
//! are documented by Microsoft: a same-volume move requested with `MOVEFILE_WRITE_THROUGH`.

use std::io;
use std::path::Path;

/// Moves `source` to a previously absent `target`, requesting write-through completion on Windows.
///
/// The Windows implementation calls `MoveFileExW` with only `MOVEFILE_WRITE_THROUGH`. It does not
/// pass `MOVEFILE_REPLACE_EXISTING` or `MOVEFILE_COPY_ALLOWED`, so retained evidence is never
/// overwritten and a cross-volume copy/delete fallback is not accepted. On non-Windows targets the
/// function returns [`io::ErrorKind::Unsupported`] before touching either path.
pub fn move_no_replace_write_through(source: &Path, target: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        windows::move_no_replace_write_through(source, target)
    }

    #[cfg(not(windows))]
    {
        let _ = (source, target);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Windows write-through move is unsupported on this platform",
        ))
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    use super::*;

    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(
            lp_existing_file_name: *const u16,
            lp_new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    pub(super) fn move_no_replace_write_through(source: &Path, target: &Path) -> io::Result<()> {
        let source = encode_path(source.as_os_str())?;
        let target = encode_path(target.as_os_str())?;

        // SAFETY: both pointers reference NUL-terminated UTF-16 buffers that remain alive for the
        // duration of this synchronous Win32 call. The function retains neither pointer.
        let moved = unsafe {
            MoveFileExW(
                source.as_ptr(),
                target.as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        };
        if moved == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn encode_path(path: &OsStr) -> io::Result<Vec<u16>> {
        let mut encoded: Vec<u16> = path.encode_wide().collect();
        if encoded.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Windows path contains an interior NUL",
            ));
        }
        encoded.push(0);
        Ok(encoded)
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::io::Write;

    use tempfile::tempdir;

    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_move_is_no_overwrite_and_preserves_bytes() {
        let root = tempdir().expect("temporary root");
        let source = root.path().join("staging.bin");
        let target = root.path().join("final.bin");
        {
            let mut file = File::create(&source).expect("create source");
            file.write_all(b"durable-evidence").expect("write source");
            file.sync_all().expect("sync source");
        }

        move_no_replace_write_through(&source, &target).expect("write-through move");
        assert!(!source.exists());
        assert_eq!(fs::read(&target).expect("read target"), b"durable-evidence");

        let second = root.path().join("second.bin");
        fs::write(&second, b"new").expect("write second source");
        let error = move_no_replace_write_through(&second, &target)
            .expect_err("existing target must reject replacement");
        assert!(
            error.raw_os_error().is_some(),
            "Win32 rejection should retain an OS error"
        );
        assert_eq!(fs::read(&target).expect("re-read target"), b"durable-evidence");
        assert_eq!(fs::read(&second).expect("source must remain"), b"new");
    }

    #[cfg(windows)]
    #[test]
    fn windows_move_accepts_unicode_paths() {
        let root = tempdir().expect("temporary root");
        let source = root.path().join("暫存-證據.bin");
        let target = root.path().join("正式-證據.bin");
        {
            let mut file = File::create(&source).expect("create unicode source");
            file.write_all(b"unicode").expect("write unicode source");
            file.sync_all().expect("sync unicode source");
        }

        move_no_replace_write_through(&source, &target).expect("move unicode path");
        assert_eq!(fs::read(&target).expect("read unicode target"), b"unicode");
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_fails_before_filesystem_access() {
        let root = tempdir().expect("temporary root");
        let source = root.path().join("does-not-exist");
        let target = root.path().join("also-does-not-exist");

        let error = move_no_replace_write_through(&source, &target)
            .expect_err("non-Windows must fail closed");
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(!source.exists());
        assert!(!target.exists());
    }
}
