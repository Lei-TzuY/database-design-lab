from pathlib import Path


def replace_once(text, old, new, label):
    if old not in text:
        raise SystemExit(f"missing marker: {label}")
    return text.replace(old, new, 1)

# Crate-level rustdoc.
path = Path("crates/db-storage-btree/src/lib.rs")
text = path.read_text()
text = replace_once(
    text,
    "//! overflow chains for values through the common 1 MiB limit. Mutations synchronize replacement and\n//! overflow pages before atomically publishing a new root. Ordered scans, 4 KiB keys, physical file\n//! compaction, and common `KvEngine` admission remain deferred.\n",
    "//! overflow chains for keys through 4 KiB and values through 1 MiB. Mutations synchronize key/value\n//! overflow pages and replacement tree pages before atomically publishing a new root. The tree now\n//! implements the common point-operation `KvEngine` contract; ordered scans, physical file compaction,\n//! and exhaustive mutation-write fault injection remain deferred.\n",
    "crate docs",
)
path.write_text(text)

# README status and persistence narrative.
path = Path("README.md")
text = path.read_text()
text = replace_once(
    text,
    "The workspace currently contains five crates with executable behavior. The B+ tree crate now includes\nan executable persistent point-operation slice, but is not yet a complete common `KvEngine`:\n",
    "The workspace currently contains five crates with executable behavior. The B+ tree is now a common\npersistent point-operation `KvEngine`; ordered scans remain a separate capability:\n",
    "README baseline intro",
)
text = replace_once(
    text,
    "| `db-storage-btree` | Fixed 4 KiB checksummed pages and mirrored superblocks plus copy-on-write binary `GET`/`PUT`/`DELETE`, root/non-root splits, delete redistribution/merge, root contraction, reachability-derived page reuse, checksummed overflow chains through 1 MiB values, reachable-tree validation, and bounded validated-page caching |",
    "| `db-storage-btree` | Common persistent point `KvEngine` with fixed 4 KiB checksummed pages, mirrored superblocks, COW `GET`/`PUT`/`DELETE`/`REOPEN`, split/rebalance/root contraction, reachability-derived page reuse, overflow-backed 4 KiB keys and 1 MiB values, reachable-tree validation, and bounded validated-page caching |",
    "README btree row",
)
text = replace_once(
    text,
    "allocation space without a persistent free-list. Values that cannot fit in a leaf are stored in\nchecksummed overflow-page chains and are included in the same reachability/reuse proof. Physical file\ncompaction/truncation, ordered scans, 4 KiB keys, and a common `KvEngine` implementation remain deferred.\n\nCurrent common semantics allow empty and arbitrary binary keys/values, cap keys at 4 KiB and values\nat 1 MiB, distinguish missing values from empty values, and expose `PUT`, `GET`, `DELETE`, and an\nexplicit `REOPEN` workload boundary. The B+ tree now matches the common 1 MiB value limit, including\nempty values, but still caps keys at 1,024 bytes rather than the common 4 KiB limit and is therefore not\nyet admitted to common differential experiments.\n",
    "allocation space without a persistent free-list. Keys that would overfill a tree cell and values\nthat cannot fit inline are stored in checksummed overflow-page chains; live key/value chains participate\nin the same reachability/reuse proof. The B+ tree implements the common 4 KiB-key/1 MiB-value point\ncontract, including explicit `REOPEN`, and is differentially tested against the in-memory oracle.\nPhysical file compaction/truncation and ordered scans remain deferred.\n\nCurrent common semantics allow empty and arbitrary binary keys/values, cap keys at 4 KiB and values\nat 1 MiB, distinguish missing values from empty values, and expose `PUT`, `GET`, `DELETE`, and an\nexplicit `REOPEN` workload boundary. Both persistent point engines now implement that contract and can\nparticipate in the common differential harness; ordered range capability remains false for the B+ tree.\n",
    "README common semantics",
)
text = text.replace(
    "A large value is first written as a tail-to-head chain of\nchecksummed overflow pages so every next-page target is already durable before its predecessor. The\nreplacement leaf then stores the logical value length and first overflow page id.",
    "A key or value that requires overflow storage is first written as a tail-to-head chain of\nchecksummed overflow pages so every next-page target is already durable before its predecessor. The\nreplacement leaf/internal cell then stores the logical length and first overflow page id.",
    1,
)
text = text.replace(
    "non-overlapping ranges, cycle/duplicate-reference rejection, canonical overflow-chain lengths, and zero\nsibling pointers for the current point-tree representation. Overflow pages referenced by live leaves are\npart of the reachable set.",
    "non-overlapping ranges, cycle/duplicate-reference rejection, canonical key/value overflow-chain\nlengths, and zero sibling pointers for the current point-tree representation. Overflow pages referenced\nby live leaves or internal separators are part of the reachable set.",
    1,
)
text = text.replace("overflow-value chains", "key/value overflow chains", 1)
path.write_text(text)

# Page-format contract.
path = Path("docs/btree-page-format.md")
text = path.read_text()
text = replace_once(
    text,
    "page reuse, and checksummed overflow chains through 1 MiB values. It is intentionally not yet a common\n`KvEngine`: physical file compaction, ordered scans, the remaining 1,024-byte key limit, and trait-level\ncapability/reopen integration remain deferred.",
    "page reuse, checksummed overflow chains for keys through 4 KiB and values through 1 MiB, and the\ncommon point-operation `KvEngine` contract. Physical file compaction, ordered scans, and exhaustive\nmutation-write fault injection remain deferred.",
    "format intro",
)
text = text.replace(
    "2. Encodes the new value inline when it fits; otherwise commits its overflow chunks tail-to-head so\n   every next-page link points to an already synchronized committed page.\n3. Builds a replacement leaf in memory with the key inserted or value replaced.",
    "2. Encodes the new key inline when its descriptor can remain page-local; otherwise commits the key\n   overflow chunks tail-to-head. Values use the same durable overflow-chain protocol when needed.\n3. Builds a replacement leaf in memory with the key inserted or value replaced. Long exact-minimum\n   keys used by internal cells are materialized through the same overflow mechanism before that parent."
)
text = text.replace(
    "Reachability includes every overflow page referenced\nby a live leaf and rejects duplicate/cyclic overflow references.",
    "Reachability includes every overflow page referenced by a live leaf key/value or internal exact-minimum\nseparator and rejects duplicate/cyclic overflow references.",
    1,
)
text = replace_once(
    text,
    "| 0 | 2 | key length |\n| 2 | 4 | value descriptor |\n| 6 | `key length` | opaque binary key |\n| ... | variable | inline bytes or one `u64` overflow-head page id |\n\nThe descriptor's high bit is the overflow flag;",
    "| 0 | 2 | key descriptor |\n| 2 | 4 | value descriptor |\n| 6 | variable | inline key bytes or one `u64` key-overflow head |\n| ... | variable | inline value bytes or one `u64` value-overflow head |\n\nThe key descriptor's high bit is the key-overflow flag and its low 15 bits are the logical key length.\nKeys through 4,096 bytes are accepted. Keys up to 4,034 bytes remain inline; the conservative threshold\nensures an inline key still leaves room for a value-overflow descriptor in an otherwise empty leaf.\nLonger keys store exactly one `u64` overflow-head page id in the cell. Because 4,096 is far below the\n15-bit marker boundary, old inline v1 key encodings remain unambiguous.\n\nThe value descriptor's high bit is the overflow flag;",
    "leaf key descriptor",
)
text = replace_once(
    text,
    "The tree now accepts values through the common 1 MiB limit. Keys remain capped at 1,024 bytes so internal\nseparator pages retain useful fanout; this remaining key mismatch is the storage-format blocker for\ncommon `KvEngine` admission.\n\n## Overflow value pages",
    "The tree therefore accepts the complete common point size contract: 4 KiB keys and 1 MiB values.\n\n## Overflow key/value pages",
    "common limits",
)
text = text.replace("An overflow page has kind `3`, exactly one non-empty slotted cell", "An overflow page has kind `3`, exactly one non-empty slotted cell", 1)
text = text.replace(
    "The single cell is raw value data; with one four-byte slot, canonical payload\ncapacity is 4,048 bytes. A value is divided with `rchunks(4048)`",
    "The single cell is raw key or value data; with one four-byte slot, canonical payload\ncapacity is 4,048 bytes. A blob is divided with `rchunks(4048)`",
    1,
)
text = text.replace(
    "Reopen derives the exact expected page count from the logical value length",
    "Reopen derives the exact expected page count from the logical key/value length",
    1,
)
text = text.replace(
    "so a live value chain can never be selected as an orphan. A failed/unpublished replacement chain\nis unreachable and may be recycled by a later mutation. The 1 MiB maximum uses at most 260 overflow\npages.",
    "so a live key or value chain can never be selected as an orphan. A failed/unpublished replacement\nchain is unreachable and may be recycled by a later mutation. A 4 KiB key uses at most two overflow\npages; the 1 MiB value maximum uses at most 260."
)
text = replace_once(
    text,
    "| 0 | 2 | separator/minimum-key length |\n| 2 | 8 | committed child page id |\n| 10 | `key length` | exact minimum key reachable from the child |",
    "| 0 | 2 | separator/minimum-key descriptor |\n| 2 | 8 | committed child page id |\n| 10 | variable | inline exact minimum key or one `u64` key-overflow head |",
    "internal key descriptor",
)
text = text.replace(
    "This representation deliberately stores the first child's minimum too,\nwhich keeps routing and structural validation uniform.",
    "This representation deliberately stores the first child's minimum too, which keeps routing and\nstructural validation uniform. A long exact minimum uses the same high-bit key descriptor and overflow\nblob format as a leaf key; routing compares reconstructed logical key bytes, so separator semantics do\nnot change when storage moves out of line.",
    1,
)
text = replace_once(
    text,
    "ordered leaf scan, 4 KiB tree keys, common `KvEngine` capability admission, multi-operation transactions,\nWAL-based in-place replacement,",
    "ordered leaf scan, multi-operation transactions, WAL-based in-place replacement,",
    "deferred list",
)
path.write_text(text)

# Design-space status.
path = Path("docs/design-space.md")
text = path.read_text()
text = text.replace(
    "| Binary KV + B+ tree + standalone | Point mutation slice implemented, common capability deferred | `db-storage-btree` has mirrored checksummed pages plus copy-on-write binary lookup/insert/delete, root/non-root splits, delete redistribution/merge, root contraction, reachability-derived orphan-page reuse, root publication, reopen validation, checksummed overflow values through 1 MiB, and deterministic split/delete/reuse/overflow/shadow-page tests; physical compaction, ordered scan, 4 KiB keys, and common `KvEngine` admission remain deferred |",
    "| Binary KV + B+ tree + standalone | Common persistent point engine implemented | `db-storage-btree` implements `KvEngine` for the full 4 KiB-key/1 MiB-value point contract with mirrored checksummed pages, COW lookup/insert/delete/reopen, split/rebalance/root contraction, reachability-derived reuse, key/value overflow blobs, and deterministic common differential tests against the memory oracle; physical compaction and ordered scan remain deferred |",
    1,
)
path.write_text(text)

# Evidence roadmap.
path = Path("docs/roadmap.md")
text = path.read_text()
text = replace_once(
    text,
    "- [ ] Admit B+ tree to the common `KvEngine` differential harness by reconciling the remaining\n  1,024-byte tree key limit with the common 4 KiB contract, wiring trait-level reopen/capabilities, and\n  adding deterministic differential regressions.\n",
    "- [x] Admit B+ tree to the common `KvEngine` differential harness. Keys through 4 KiB use inline\n  descriptors or checksummed overflow blobs in leaves and internal exact-minimum separators; trait-level\n  capabilities/reopen match the common contract, and deterministic differential tests cover empty/binary\n  keys, the 4 KiB/1 MiB size limits, overwrite, delete, and repeated reopen against the memory oracle.\n",
    "roadmap common admission",
)
path.write_text(text)
