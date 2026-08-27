# B+ tree page-file format v1

Format v1 uses immutable checksummed 4,096-byte pages plus mirrored metadata. The executable tree
layer now defines binary leaf/internal cells, point lookup, copy-on-write insertion/update/deletion,
root/non-root split propagation, deletion redistribution/merge, root contraction, reachability-derived
page reuse, and checksummed overflow chains through 1 MiB values. It is intentionally not yet a common
`KvEngine`: physical file compaction, ordered scans, the remaining 1,024-byte key limit, and trait-level
capability/reopen integration remain deferred. All integers are unsigned little-endian unless stated otherwise.

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

## Copy-on-write tree mutation

A successful tree mutation does not modify any page reachable from the currently published root.
`PUT` performs:

1. Searches from the current root to the target leaf.
2. Encodes the new value inline when it fits; otherwise commits its overflow chunks tail-to-head so
   every next-page link points to an already synchronized committed page.
3. Builds a replacement leaf in memory with the key inserted or value replaced.
4. If a tree page overflows, divides its sorted cells into two independently valid pages.
5. Commits replacement leaf/split pages as immutable allocations.
6. Rebuilds the parent with replacement child references; parent overflow recursively produces two
   replacement internal pages, propagating toward the root.
7. If propagation returns two top-level pages, commits a new two-child internal root.
8. Only after every overflow and replacement page has been synchronized and committed, publishes the
   final root page id with one more mirrored-superblock generation.

`DELETE` follows the same publication boundary. It first confirms the key exists, so deleting a
missing key is a true no-op with no page allocation or metadata generation. The changed leaf is copied
without the target entry. If that child becomes underfull, the parent combines it with an adjacent
sibling: when all encoded cells fit on one page the pair merges; otherwise cells are redistributed by
encoded byte size. Internal redistribution requires at least two children on each resulting sibling.
An empty child disappears from its parent, a one-child top-level internal page contracts to its only
child, and deleting the final leaf entry publishes `root = 0`. Replacement pages are synchronized
before the final root-pointer transition exactly as for `PUT`.

A crash before the final root publication leaves the old root authoritative. Overflow and tree pages
already committed during the earlier steps are
valid but unreachable copy-on-write history. A torn new-root superblock falls back to the prior valid
metadata generation, which still names the old root. A crash after the new root metadata is durable
finds every referenced replacement page already synchronized. This protocol therefore avoids an
in-place torn-page update problem; it does **not** reclaim unreachable historical pages yet.

The deterministic test suite also constructs a committed-but-unpublished shadow page and verifies that
reopen continues to return the value reachable through the older root.

## Reachability-derived page reuse

Before each mutating tree operation, the currently published root is structurally validated while its
reachable page ids are collected. Every committed data-page id outside that set is an orphan from an
earlier successful COW publication (or an unpublished failed/shadow attempt) and is eligible for reuse.
The current root cannot reference such a page. Reachability includes every overflow page referenced
by a live leaf and rejects duplicate/cyclic overflow references. Point-tree v1 additionally requires
reachable leaf right-sibling fields to be zero so no hidden tree edge escapes the reachability walk.

A recycled page is rebuilt with the same physical page id, fully overwritten, and `sync_data` is called
before it may appear beneath a newly published root. Recycling does not change `page_count`: the extent
was already committed. If the recycled write tears or the process crashes before root publication, the
old root still does not reference that page, so reopen validates the old tree and the orphan id can be
reused again. If the new root becomes durable, every recycled page it references was synchronized first.
This derives free space from reachability instead of persisting a separate free-list, avoiding a second
metadata structure that would itself require atomic update rules. The physical file does not shrink;
compaction/truncation is a separate future concern.

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
| 5 | 1 | kind | `1` leaf, `2` internal, `3` overflow |
| 6 | 2 | flags | zero |
| 8 | 8 | encoded physical page id | must equal its physical position |
| 16 | 8 | page generation | reserved as zero in v1 |
| 24 | 2 | lower free-space boundary | exactly header + slot-array bytes |
| 26 | 2 | upper free-space boundary | at most byte 4092 |
| 28 | 2 | cell count | defines exactly that many slots |
| 30 | 2 | reserved | zero |
| 32 | 8 | kind-specific page link | leaf: right sibling (point tree requires zero); overflow: next chunk page; internal: zero |
| 40 | variable | four-byte slots | `(u16 offset, u16 length)` |
| ... | variable | zeroed free space | `lower..upper` |
| ... | variable | packed cell payloads | contiguous through byte 4091 |
| 4092 | 4 | page CRC-32 | exact match |

The pager verifies checksum, magic, version, flags, physical page id, reserved fields, free-space
boundaries, slot count, nonzero cell lengths, payload extents, and exact non-overlapping packed payload
coverage before returning a page. Internal pages cannot carry a page link. Any nonzero leaf/overflow
link must avoid the mirrored superblocks and self-reference, and pager reads require the target to be
inside the committed page extent.

## Leaf cells

Reachable leaf cells are sorted in strictly increasing bytewise key order. Empty keys and empty values
are legal. Duplicate keys are not: `PUT` replaces the value while rebuilding the copy-on-write path.

| Cell offset | Bytes | Meaning |
| ---: | ---: | --- |
| 0 | 2 | key length |
| 2 | 4 | value descriptor |
| 6 | `key length` | opaque binary key |
| ... | variable | inline bytes or one `u64` overflow-head page id |

The descriptor's high bit is the overflow flag; the low 31 bits encode the logical value length. When
the high bit is clear, exactly that many value bytes follow the key, preserving compatibility with the
previous inline v1 encoding. When the high bit is set, the logical length must be `1..=1,048,576` and
exactly eight bytes follow the key containing the first overflow page id. Because the common value limit
is only 1 MiB, the marker bit cannot collide with a previously legal inline length. New readers can open
older inline-only v1 files; older binaries intentionally reject kind-3 pages in files that use overflow.

The tree now accepts values through the common 1 MiB limit. Keys remain capped at 1,024 bytes so internal
separator pages retain useful fanout; this remaining key mismatch is the storage-format blocker for
common `KvEngine` admission.

## Overflow value pages

An overflow page has kind `3`, exactly one non-empty slotted cell, and uses header bytes 32–39 as an
optional next-page id. The single cell is raw value data; with one four-byte slot, canonical payload
capacity is 4,048 bytes. A value is divided with `rchunks(4048)` and pages are committed tail-to-head,
therefore every nonzero next link names a page that was already synchronized before its predecessor.
The leaf is committed only after the full chain exists, and the new root is published only after the
leaf and all rewritten ancestors are durable.

Reopen derives the exact expected page count from the logical value length, requires the first chunk to
hold the remainder and every later chunk to hold exactly 4,048 bytes, requires exactly one cell per
overflow page, rejects short/long chains, kind mismatches, cycles, duplicate references, invalid links,
and total-length mismatches. Overflow pages are added to the same reachable-page set used by COW space
reuse, so a live value chain can never be selected as an orphan. A failed/unpublished replacement chain
is unreachable and may be recycled by a later mutation. The 1 MiB maximum uses at most 260 overflow
pages.

## Internal cells

Every internal cell describes one child and the exact minimum key reachable in that child. Cells are
sorted by that minimum key. This representation deliberately stores the first child's minimum too,
which keeps routing and structural validation uniform.

| Cell offset | Bytes | Meaning |
| ---: | ---: | --- |
| 0 | 2 | separator/minimum-key length |
| 2 | 8 | committed child page id |
| 10 | `key length` | exact minimum key reachable from the child |

Lookup chooses the child whose minimum key is the greatest key not greater than the search key; a
search smaller than the first separator descends into the first child and naturally returns missing at
the leaf. Reachable internal pages have at least two children. Reopen recursively verifies that each
stored separator equals the referenced child's actual minimum key, child ranges do not overlap, all
children have equal height, and no page is reached twice or through a cycle. Traversal depth is bounded
to fail closed on malicious/corrupt structures.

## Slotted-page behavior

`Page::insert_cell` stores one non-empty encoded cell. Slots grow upward from byte 40 while payloads
grow downward from byte 4092. An insertion is rejected before mutation when the slot plus payload cannot
fit. The page checksum is refreshed after each successful in-memory edit.

`Pager::prepare_new_page` assigns the next candidate physical id. `Pager::commit_new_page` persists that
page with the allocation ordering above. `Pager::read_page` validates before caching. The cache is
bounded and stores only immutable validated page images, so eviction has no dirty-page writeback
semantics.

## Explicitly deferred

Format/tree work still does not define physical file compaction/truncation, a copy-on-write-compatible
ordered leaf scan, 4 KiB tree keys, common `KvEngine` capability admission, multi-operation transactions,
WAL-based in-place replacement,
or concurrent writers. Fault injection for every intermediate mutation write is also still required
before performance experiments; current evidence covers pager torn/truncated states plus the semantic
unpublished-shadow-root case.
