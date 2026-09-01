# Windows write-through namespace primitive

The repository's append-log generation lifecycle intentionally fails closed on Windows wherever it would otherwise need to claim durable namespace publication without an implemented mechanism. `db_cli::windows_durable::move_no_replace_write_through` is the first narrow platform primitive intended to reduce that gap.

## Contract

On Windows the function calls `MoveFileExW` with **only** `MOVEFILE_WRITE_THROUGH`.

That deliberately means:

- the source is moved to the target name;
- the call requests write-through completion before returning success;
- `MOVEFILE_REPLACE_EXISTING` is not enabled, so an existing target is not overwritten;
- `MOVEFILE_COPY_ALLOWED` is not enabled, so the function does not accept a cross-volume copy/delete fallback;
- paths are passed as NUL-terminated UTF-16 and Unicode path behavior is covered by the Windows test suite.

On non-Windows targets the function returns `Unsupported` before filesystem access.

The raw Win32 FFI is isolated to this module. The rest of `db-cli` keeps `unsafe_code = "deny"`; `lib.rs` grants a local allowance only for this audited boundary.

## Why this is not yet Windows generation durability

This primitive solves only one namespace transition: **an already-synchronized staging object to a previously absent canonical name**. Existing Unix generation protocols also rely on other durable operations, including generation construction, reservation creation, marker publication, cleanup, migration, and cutover ordering.

No upper-level generation protocol uses this Windows primitive yet. The current Windows fail-before-write behavior for unsupported durable generation publication/reservation/cleanup/migration/cutover remains unchanged until each caller is explicitly redesigned around a Windows-safe ordering.

## Evidence boundary

Windows CI must prove the actual API call succeeds for a fresh target, rejects replacement without altering either retained object, and handles Unicode paths. That is an executable API/contract check on the hosted Windows filesystem.

It is **not** physical power-loss testing and does not claim that every Windows filesystem, storage controller, or device honors persistence identically. The project will only lift an upper-level Windows fail-closed boundary after the complete operation ordering is defensible, not merely because this primitive exists.
