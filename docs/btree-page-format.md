# B+ tree page-file format v1

This phase establishes the physical page and pager contract before implementing tree search or mutation.
It is intentionally not yet a `KvEngine`. All integers are unsigned little-endian and every physical
page is exactly 4,096 bytes.

## Commit model

Pages 0 and 1 are mirrored superblocks. Each valid superblock contains a monotonically increasing
`generation`, the committed total `page_count`, and an optional root page id. The valid copy with the
highest generation is authoritative. Equal-generation copies must describe identical metadata, and two
valid copies may differ by at most one generation because metadata writes alternate slots.

A new immutable data page is committed in this order:

1. Encode the complete checksummed page at physical page id `page_count`.
2. Append the 4 KiB page and call `sync_data`.
3. Encode `page_count + 1` and `generation + 1` into the inactive superblock slot.
4. Write the complete superblock page and call `sync_data`.
5. Only after both operations succeed does the live pager expose the new metadata generation.

If page append fails or the superblock update has an I/O error, the live pager is poisoned because the
commit outcome is ambiguous. It must be reopened. On reopen, the newest valid superblock decides the
committed extent. Up to one physical page beyond that extent is an interrupted allocation and is
truncated; those bytes were never committed by metadata. More than one trailing page fails closed.
If the selected superblock commits bytes that are physically missing, open fails closed.

This protects page *allocation* and root-pointer metadata from a torn newer superblock. It does not yet
make arbitrary in-place tree-page replacement crash-safe. Therefore format v1 deliberately exposes no
in-place pager write API. A later insertion/split phase must choose and test a tree mutation protocol
before page replacement can be acknowledged as durable.

## Superblocks: pages 0 and 1

The checksum is CRC-32 (IEEE) over bytes 0–4091. Bytes 4092–4095 store the checksum itself.

| Offset | Bytes | Meaning | v1 validation |
| ---: | ---: | --- | --- |
| 0 | 8 | magic `DBBPTRE\0` | exact match |
| 8 | 2 | format version | `1` |
| 10 | 2 | physical page size | `4096` |
| 12 | 1 | superblock slot id | equals physical slot 0 or 1 |
| 13 | 3 | reserved | all zero |
| 16 | 8 | metadata generation | monotonic across committed metadata writes |
| 24 | 8 | total committed page count | at least `2` |
| 32 | 8 | root page id | `0` for none, otherwise `2 <= root < page_count` |
| 40 | 4 | flags | zero |
| 44 | 4048 | reserved | all zero |
| 4092 | 4 | page CRC-32 | exact match |

A single invalid superblock is recoverable when the other copy validates. If neither copy validates,
open fails. The two copies are not a general journal: they protect the small committed metadata state
whose update follows already-synchronized immutable page allocation.

## Data-page header

Data page ids begin at 2. A page checksum again covers bytes 0–4091.

| Offset | Bytes | Meaning | v1 validation |
| ---: | ---: | --- | --- |
| 0 | 4 | magic `BTPG` | exact match |
| 4 | 1 | format version | `1` |
| 5 | 1 | kind | `1` leaf, `2` internal |
| 6 | 2 | flags | zero |
| 8 | 8 | encoded physical page id | must equal its physical position |
| 16 | 8 | page generation | reserved as zero in v1 |
| 24 | 2 | lower free-space boundary | exactly header + slot-array bytes |
| 26 | 2 | upper free-space boundary | at most byte 4092 |
| 28 | 2 | cell count | defines exactly that many slots |
| 30 | 2 | reserved | zero |
| 32 | 8 | right sibling | leaf-only; `0` means none |
| 40 | variable | four-byte slots | `(u16 offset, u16 length)` |
| ... | variable | zeroed free space | `lower..upper` |
| ... | variable | packed opaque cell payloads | contiguous through byte 4091 |
| 4092 | 4 | page CRC-32 | exact match |

The pager verifies checksum, magic, version, flags, physical page id, reserved fields, free-space
boundaries, slot count, nonzero cell lengths, payload extents, and exact non-overlapping packed payload
coverage before returning a page. Internal pages cannot carry a leaf sibling pointer. A leaf sibling
read through the pager must refer to an already committed data page.

## Slotted-page behavior implemented now

`Page::insert_cell` stores one opaque non-empty byte string without interpreting B+ tree keys. Slots
grow upward from byte 40 while payloads grow downward from byte 4092. An insertion is rejected before
mutation when the slot plus payload cannot fit. The page checksum is refreshed after each successful
in-memory edit.

`Pager::prepare_new_page` assigns the next candidate physical id. `Pager::commit_new_page` persists that
page with the commit ordering above. `Pager::read_page` validates before caching. The cache is bounded
and stores only immutable validated page images, so eviction has no dirty-page writeback semantics.

## Explicitly deferred

This page layer does not yet define leaf key/value encoding, internal separator/child encoding, binary
search, root/non-root splits, deletion/merge, free-list reuse, copy-on-write tree generations, WAL,
in-place page replacement, ordered scans, or a `KvEngine` implementation. Those features must preserve
this fail-closed validation discipline and add deterministic crash-state tests before performance work.
