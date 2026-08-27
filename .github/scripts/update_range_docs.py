from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        raise SystemExit(f"missing marker: {label}")
    return text.replace(old, new, 1)

readme = Path("README.md")
text = readme.read_text()
text = replace_once(
    text,
    "The workspace currently contains five crates with executable behavior. The B+ tree is now a common\npersistent point-operation `KvEngine`; ordered scans remain a separate capability:\n",
    "The workspace currently contains five crates with executable behavior. The B+ tree is now a common\npersistent `KvEngine` for point operations and bounded ordered range scans:\n",
    "README baseline",
)
text = replace_once(
    text,
    "| `db-storage-btree` | Common persistent point `KvEngine` with fixed 4 KiB checksummed pages, mirrored superblocks, COW `GET`/`PUT`/`DELETE`/`REOPEN`, split/rebalance/root contraction, reachability-derived page reuse, overflow-backed 4 KiB keys and 1 MiB values, reachable-tree validation, and bounded validated-page caching |",
    "| `db-storage-btree` | Common persistent `KvEngine` with fixed 4 KiB checksummed pages, mirrored superblocks, COW `GET`/`PUT`/`DELETE`/`REOPEN`, half-open ordered `range_scan`, split/rebalance/root contraction, reachability-derived page reuse, overflow-backed 4 KiB keys and 1 MiB values, reachable-tree validation, and bounded validated-page caching |",
    "README crate role",
)
text = replace_once(
    text,
    "contract, including explicit `REOPEN`, and is differentially tested against the in-memory oracle.\nPhysical file compaction/truncation and ordered scans remain deferred.\n\nCurrent common semantics allow empty and arbitrary binary keys/values, cap keys at 4 KiB and values\nat 1 MiB, distinguish missing values from empty values, and expose `PUT`, `GET`, `DELETE`, and an\nexplicit `REOPEN` workload boundary. Both persistent point engines now implement that contract and can\nparticipate in the common differential harness; ordered range capability remains false for the B+ tree.\nRange scans, transactions, multi-process writers, compaction, replication, SQL, MVCC, Raft, graph,\ntime-series, and columnar execution are not implemented.\n",
    "contract, including explicit `REOPEN`, and is differentially tested against the in-memory oracle.\nOrdered B+ tree scans walk the internal hierarchy in key order rather than persisting leaf sibling\nlinks, so scans introduce no extra COW/reuse edge. Physical file compaction/truncation remains deferred.\n\nCurrent common semantics allow empty and arbitrary binary keys/values, cap keys at 4 KiB and values\nat 1 MiB, distinguish missing values from empty values, and expose `PUT`, `GET`, `DELETE`, `REOPEN`,\nand a bounded half-open ordered range API `[start, end)`, with `end = None` meaning unbounded. The\nin-memory oracle and B+ tree advertise ordered range support; the append log deliberately does not,\nbecause its replay `BTreeMap` is not an on-disk ordered access path. Workload schema v1 still serializes\npoint/lifecycle steps only; reproducible generated range traces remain Phase 4 work. Transactions,\nmulti-process writers, compaction, replication, SQL, MVCC, Raft, graph, time-series, and columnar\nexecution are not implemented.\n",
    "README ordered semantics",
)
readme.write_text(text)

lib = Path("crates/db-storage-btree/src/lib.rs")
text = lib.read_text()
text = replace_once(
    text,
    "//! implements the common point-operation `KvEngine` contract; ordered scans, physical file compaction,\n//! and exhaustive mutation-write fault injection remain deferred.\n",
    "//! implements the common `KvEngine` point contract plus bounded half-open ordered scans by walking\n//! internal children in key order; physical file compaction and exhaustive mutation-write fault\n//! injection remain deferred.\n",
    "btree crate docs",
)
lib.write_text(text)

page = Path("docs/btree-page-format.md")
text = page.read_text()
text = replace_once(
    text,
    "page reuse, checksummed overflow chains for keys through 4 KiB and values through 1 MiB, and the\ncommon point-operation `KvEngine` contract. Physical file compaction, ordered scans, and exhaustive\nmutation-write fault injection remain deferred. All integers are unsigned little-endian unless stated otherwise.\n",
    "page reuse, checksummed overflow chains for keys through 4 KiB and values through 1 MiB, the\ncommon point-operation `KvEngine` contract, and bounded half-open ordered range scans. Physical file\ncompaction and exhaustive mutation-write fault injection remain deferred. All integers are unsigned\nlittle-endian unless stated otherwise.\n",
    "page format intro",
)
text = replace_once(
    text,
    "stored separator equals the referenced child's actual minimum key, child ranges do not overlap, all\nchildren have equal height, and no page is reached twice or through a cycle. Traversal depth is bounded\nto fail closed on malicious/corrupt structures.\n\n## Slotted-page behavior\n",
    "stored separator equals the referenced child's actual minimum key, child ranges do not overlap, all\nchildren have equal height, and no page is reached twice or through a cycle. Traversal depth is bounded\nto fail closed on malicious/corrupt structures.\n\n## Ordered range scans\n\n`range_scan(start, end, limit)` returns at most `limit` live entries in ascending bytewise key order.\nThe lower bound is inclusive, the optional upper bound is exclusive, and `end = None` is unbounded.\nEqual bounds and a zero limit return an empty result; an upper bound that sorts before the lower bound\nis invalid input. Scans are read-only and do not change metadata generation or committed page count.\n\nFormat v1 deliberately continues to require zero leaf right-sibling fields. A scan instead descends\nthrough internal children in separator order. For child `i`, the exact minimum of child `i + 1` is an\nexclusive upper bound on every key in child `i`; if that next minimum is less than or equal to `start`,\nchild `i` can be skipped. Traversal stops globally when a child minimum reaches `end`, a leaf key reaches\n`end`, or `limit` rows have been emitted. This preserves ordered semantics without introducing sibling\nreferences that copy-on-write mutation and orphan-page reachability would also have to rewrite/track.\nValues, including overflow-backed values, are materialized only for rows that survive the range bounds.\n\n## Slotted-page behavior\n",
    "page format scan section",
)
text = replace_once(
    text,
    "Format/tree work still does not define physical file compaction/truncation, a copy-on-write-compatible\nordered leaf scan, multi-operation transactions, WAL-based in-place replacement,\nor concurrent writers. Fault injection for every intermediate mutation write is also still required\n",
    "Format/tree work still does not define physical file compaction/truncation, multi-operation\ntransactions, WAL-based in-place replacement, or concurrent writers. Fault injection for every\nintermediate mutation write is also still required\n",
    "page format deferred",
)
page.write_text(text)

design = Path("docs/design-space.md")
text = design.read_text()
text = replace_once(
    text,
    "| Binary KV + B+ tree + standalone | Common persistent point engine implemented | `db-storage-btree` implements `KvEngine` for the full 4 KiB-key/1 MiB-value point contract with mirrored checksummed pages, COW lookup/insert/delete/reopen, split/rebalance/root contraction, reachability-derived page reuse, key/value overflow blobs, and deterministic common differential tests against the memory oracle; physical compaction and ordered scan remain deferred |",
    "| Binary KV + B+ tree + standalone | Common persistent point + ordered-range engine implemented | `db-storage-btree` implements `KvEngine` for the full 4 KiB-key/1 MiB-value point contract plus bounded half-open ordered scans. Scans traverse internal children in separator order without leaf sibling links and are differentially checked against the memory oracle after splits, reopen, delete, and overflow-value materialization; physical compaction remains deferred |",
    "design-space B+ tree row",
)
design.write_text(text)

roadmap = Path("docs/roadmap.md")
text = roadmap.read_text()
text = replace_once(
    text,
    "- [ ] Expose true ordered scans through the common capability contract, including a sibling/link or\n  traversal design compatible with copy-on-write mutation.\n",
    "- [x] Expose true ordered scans through the common capability contract using bounded half-open\n  `[start, end)` traversal over internal child order rather than leaf sibling links. Exact child minima\n  prune pre-start subtrees and stop at the upper bound; tests prove sorted/limited/read-only behavior,\n  reopen/delete/overflow-value correctness, and equality with the in-memory oracle.\n",
    "roadmap ordered scan",
)
roadmap.write_text(text)

constitution = Path("docs/experimental-constitution.md")
text = constitution.read_text()
text = replace_once(
    text,
    "| `DELETE(k)` | remove `k`; the log engine appends a tombstone even for a miss | the removed value or missing |\n| `REOPEN` | close/reconstruct engine state | successful lifecycle boundary |\n",
    "| `DELETE(k)` | remove `k`; the log engine appends a tombstone even for a miss | the removed value or missing |\n| `RANGE(start, end, limit)` | none | up to `limit` live pairs in bytewise key order from `[start, end)`; `end` may be unbounded |\n| `REOPEN` | close/reconstruct engine state | successful lifecycle boundary |\n",
    "constitution operation table",
)
text = replace_once(
    text,
    "distinct. Keys are at most 4,096 bytes and values at most 1,048,576 bytes. There is no implicit text\nencoding, ordering API, transaction group, snapshot, TTL, compare-and-swap, or concurrency guarantee\nin the current common semantics. JSON workload bytes use even-length hexadecimal strings.\n\nWorkload schema version 1 records an optional seed and ordered steps. The built-in generator uses a\n",
    "distinct. Keys are at most 4,096 bytes and values at most 1,048,576 bytes. Ordered range bounds use\nthe same bytewise ordering as B+ tree routing: the lower bound is inclusive, the upper bound exclusive,\n`end = None` is unbounded, equal bounds are empty, and `end < start` is invalid. There is no implicit\ntext encoding, transaction group, snapshot, TTL, compare-and-swap, or concurrency guarantee.\n\nWorkload schema version 1 records an optional seed and ordered point/lifecycle steps; it does not yet\nserialize range scans. The built-in generator uses a\n",
    "constitution semantics",
)
constitution.write_text(text)
