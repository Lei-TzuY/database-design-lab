from pathlib import Path

lib = Path("crates/db-storage-btree/src/lib.rs")
text = lib.read_text()
text = text.replace(
    "//! validated-page cache form the physical layer. The tree layer adds binary point lookup, insertion/\n//! update, and root/non-root split propagation. Mutations append replacement pages before atomically\n//! publishing a new root; deletion, reclamation, ordered scans, overflow values, and common `KvEngine`\n//! admission remain deferred.",
    "//! validated-page cache form the physical layer. The tree layer adds binary point lookup, insertion/\n//! update/deletion, split/rebalance/root contraction, reachability-derived page reuse, and checksummed\n//! overflow chains for values through the common 1 MiB limit. Mutations synchronize replacement and\n//! overflow pages before atomically publishing a new root. Ordered scans, 4 KiB keys, physical file\n//! compaction, and common `KvEngine` admission remain deferred.",
    1,
)
text = text.replace(
    "    /// Leaf page; later phases will encode key/value cells here.",
    "    /// Leaf page containing sorted key/value cells or overflow-value references.",
    1,
)
lib.write_text(text)

tree = Path("crates/db-storage-btree/src/tree.rs")
text = tree.read_text().replace(
    "    /// One encoded entry must fit on an otherwise empty leaf. Before writing, the tree derives a\n    /// reusable-page pool from committed pages not reachable from the current root; only those orphan\n    /// pages may be overwritten before the final copy-on-write root publication.",
    "    /// Values through 1 MiB are accepted. Values that do not fit inline are stored in checksummed\n    /// overflow pages before the replacement leaf is committed. Before writing, the tree derives a\n    /// reusable-page pool from committed pages not reachable from the current root; only those orphan\n    /// pages may be overwritten before the final copy-on-write root publication.",
    1,
)
tree.write_text(text)

readme = Path("README.md")
text = readme.read_text()
text = text.replace(
    "| `db-storage-btree` | Fixed 4 KiB checksummed pages and mirrored superblocks plus copy-on-write binary `GET`/`PUT`/`DELETE`, root/non-root splits, byte-aware delete redistribution/merge, root contraction, reachability-derived orphan-page reuse, reachable-tree validation, and bounded validated-page caching |",
    "| `db-storage-btree` | Fixed 4 KiB checksummed pages and mirrored superblocks plus copy-on-write binary `GET`/`PUT`/`DELETE`, root/non-root splits, delete redistribution/merge, root contraction, reachability-derived page reuse, checksummed overflow chains through 1 MiB values, reachable-tree validation, and bounded validated-page caching |",
    1,
)
text = text.replace(
    "allocation space without a persistent free-list. Physical file compaction/truncation, ordered scans,\nlarge overflow values, and a common `KvEngine` implementation remain deferred.",
    "allocation space without a persistent free-list. Values that cannot fit in a leaf are stored in\nchecksummed overflow-page chains and are included in the same reachability/reuse proof. Physical file\ncompaction/truncation, ordered scans, 4 KiB keys, and a common `KvEngine` implementation remain deferred.",
    1,
)
text = text.replace(
    "explicit `REOPEN` workload boundary. The current B+ tree point slice intentionally has a narrower\npage-local key/value bound and therefore is not yet admitted to common differential experiments.",
    "explicit `REOPEN` workload boundary. The B+ tree now matches the common 1 MiB value limit, including\nempty values, but still caps keys at 1,024 bytes rather than the common 4 KiB limit and is therefore not\nyet admitted to common differential experiments.",
    1,
)
text = text.replace(
    "ancestor are appended as immutable pages. Root/non-root overflows split into new pages; deletion\nremoves the target entry, merges sibling pages when their combined encoded cells fit, otherwise",
    "ancestor are appended as immutable pages. A large value is first written as a tail-to-head chain of\nchecksummed overflow pages so every next-page target is already durable before its predecessor. The\nreplacement leaf then stores the logical value length and first overflow page id. Root/non-root tree\noverflows split into new pages; deletion removes the target entry, merges sibling pages when their\ncombined encoded cells fit, otherwise",
    1,
)
text = text.replace(
    "including sorted leaf keys, ordered separators, separator/child-minimum agreement, equal child heights,\nnon-overlapping ranges, cycle/duplicate-reference rejection, and zero sibling pointers for the current\npoint-tree representation. Before a mutation, every committed page absent from that validated reachable",
    "including sorted leaf keys, ordered separators, separator/child-minimum agreement, equal child heights,\nnon-overlapping ranges, cycle/duplicate-reference rejection, canonical overflow-chain lengths, and zero\nsibling pointers for the current point-tree representation. Overflow pages referenced by live leaves are\npart of the reachable set. Before a mutation, every committed page absent from that validated reachable",
    1,
)
text = text.replace(
    "  insert/delete publication, split/rebalance behavior, validation, and current crash-state limits.",
    "  insert/delete publication, overflow-value chains, split/rebalance behavior, validation, and current\n  crash-state limits.",
    1,
)
readme.write_text(text)

roadmap = Path("docs/roadmap.md")
text = roadmap.read_text()
needle = "- [x] Reclaim unreachable copy-on-write history as reusable allocation space by deriving orphan page\n  ids from validated current-root reachability before each mutation. Recycled pages are synchronized\n  before root publication, never overwrite the authoritative tree, and deterministic tests prove file\n  page count stabilizes across repeated updates and empty-tree reuse. Physical file compaction remains deferred.\n"
addition = needle + "- [x] Support values through the common 1 MiB limit with checksummed overflow-page chains. Leaf\n  references preserve logical length, overflow pages are committed tail-to-head before leaf/root\n  publication, reopen validates canonical chains, and deterministic tests cover 1 MiB round-trip,\n  delete, and orphan-page reuse without unbounded page-count growth.\n"
if needle in text and "Support values through the common 1 MiB limit" not in text:
    text = text.replace(needle, addition, 1)
text = text.replace(
    "- [ ] Admit B+ tree to the common `KvEngine` differential harness by reconciling page-local key/value\n  limits, implementing delete/reopen semantics, and declaring explicit capabilities.",
    "- [ ] Admit B+ tree to the common `KvEngine` differential harness by reconciling the remaining\n  1,024-byte tree key limit with the common 4 KiB contract, wiring trait-level reopen/capabilities, and\n  adding deterministic differential regressions.",
    1,
)
roadmap.write_text(text)

design = Path("docs/design-space.md")
text = design.read_text().replace(
    "root contraction, reachability-derived orphan-page reuse, root publication, reopen validation, and deterministic split/delete/reuse/shadow-page tests; physical compaction, ordered scan, overflow values, and common `KvEngine` admission remain deferred |",
    "root contraction, reachability-derived orphan-page reuse, checksummed overflow chains through 1 MiB values, root publication, reopen validation, and deterministic split/delete/reuse/overflow/shadow-page tests; physical compaction, ordered scan, the remaining 1,024-byte key limit, and common `KvEngine` admission remain deferred |",
    1,
)
design.write_text(text)

page = Path("docs/btree-page-format.md")
text = page.read_text()
text = text.replace(
    "root/non-root split propagation, deletion redistribution/merge, and root contraction. It is intentionally\nnot yet a common `KvEngine`: physical file compaction, ordered scans, overflow values, and common-contract\nsize limits remain deferred. All\nintegers are unsigned little-endian unless stated otherwise.",
    "root/non-root split propagation, deletion redistribution/merge, root contraction, reachability-derived\npage reuse, and checksummed overflow chains through 1 MiB values. It is intentionally not yet a common\n`KvEngine`: physical file compaction, ordered scans, the remaining 1,024-byte key limit, and trait-level\ncapability/reopen integration remain deferred. All integers are unsigned little-endian unless stated otherwise.",
    1,
)
text = text.replace(
    "1. Searches from the current root to the target leaf.\n2. Builds a replacement leaf in memory with the key inserted or value replaced.\n3. If a page overflows, divides its sorted cells into two independently valid pages.\n4. Commits replacement leaf/split pages as immutable allocations.",
    "1. Searches from the current root to the target leaf.\n2. Encodes the new value inline when it fits; otherwise commits its overflow chunks tail-to-head so\n   every next-page link points to an already synchronized committed page.\n3. Builds a replacement leaf in memory with the key inserted or value replaced.\n4. If a tree page overflows, divides its sorted cells into two independently valid pages.\n5. Commits replacement leaf/split pages as immutable allocations.",
    1,
)
text = text.replace(
    "5. Rebuilds the parent with replacement child references; parent overflow recursively produces two\n   replacement internal pages, propagating toward the root.\n6. If propagation returns two top-level pages, commits a new two-child internal root.\n7. Only after every new page has been synchronized and committed to `page_count`, publishes the final\n   root page id with one more mirrored-superblock generation.",
    "6. Rebuilds the parent with replacement child references; parent overflow recursively produces two\n   replacement internal pages, propagating toward the root.\n7. If propagation returns two top-level pages, commits a new two-child internal root.\n8. Only after every overflow and replacement page has been synchronized and committed, publishes the\n   final root page id with one more mirrored-superblock generation.",
    1,
)
text = text.replace(
    "A crash before step 7 leaves the old root authoritative. Pages already committed during steps 4–6 are",
    "A crash before the final root publication leaves the old root authoritative. Overflow and tree pages\nalready committed during the earlier steps are",
    1,
)
text = text.replace(
    "The current root cannot reference such a page; point-tree v1 additionally requires reachable leaf\nright-sibling fields to be zero so no hidden tree edge escapes the reachability walk.",
    "The current root cannot reference such a page. Reachability includes every overflow page referenced\nby a live leaf and rejects duplicate/cyclic overflow references. Point-tree v1 additionally requires\nreachable leaf right-sibling fields to be zero so no hidden tree edge escapes the reachability walk.",
    1,
)
text = text.replace(
    "| 5 | 1 | kind | `1` leaf, `2` internal |",
    "| 5 | 1 | kind | `1` leaf, `2` internal, `3` overflow |",
    1,
)
text = text.replace(
    "| 32 | 8 | right sibling | leaf-only physical field; tree point slice currently leaves it zero |",
    "| 32 | 8 | kind-specific page link | leaf: right sibling (point tree requires zero); overflow: next chunk page; internal: zero |",
    1,
)
text = text.replace(
    "coverage before returning a page. Internal pages cannot carry a leaf sibling pointer. A nonzero leaf\nsibling read through the pager must refer to an already committed data page.",
    "coverage before returning a page. Internal pages cannot carry a page link. Any nonzero leaf/overflow\nlink must avoid the mirrored superblocks and self-reference, and pager reads require the target to be\ninside the committed page extent.",
    1,
)
old_leaf = """| Cell offset | Bytes | Meaning |
| ---: | ---: | --- |
| 0 | 2 | key length |
| 2 | 4 | value length |
| 6 | `key length` | opaque binary key |
| ... | `value length` | opaque binary value |

The current tree layer caps keys at 1,024 bytes so internal separator pages retain a useful minimum
fanout. A complete encoded key/value cell plus its four-byte slot must fit on one empty page; overflow
value pages are not implemented. These bounds are intentionally narrower than `db-core`'s common
4 KiB-key/1 MiB-value contract, which is why this slice is not yet exposed as a common `KvEngine`.
"""
new_leaf = """| Cell offset | Bytes | Meaning |
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
"""
if old_leaf in text:
    text = text.replace(old_leaf, new_leaf, 1)
else:
    raise SystemExit("leaf format section marker not found")
text = text.replace(
    "Format/tree work still does not define physical file compaction/truncation, overflow values, a\ncopy-on-write-compatible ordered leaf scan,\ncommon `KvEngine` capability admission, multi-operation transactions, WAL-based in-place replacement,",
    "Format/tree work still does not define physical file compaction/truncation, a copy-on-write-compatible\nordered leaf scan, 4 KiB tree keys, common `KvEngine` capability admission, multi-operation transactions,\nWAL-based in-place replacement,",
    1,
)
page.write_text(text)
