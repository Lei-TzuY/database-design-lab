from pathlib import Path


def replace_once(path: str, old: str, new: str, label: str) -> None:
    file = Path(path)
    text = file.read_text()
    if old not in text:
        raise SystemExit(f"missing marker {label} in {path}")
    file.write_text(text.replace(old, new, 1))


replace_once(
    "crates/db-storage-btree/src/lib.rs",
    "//! internal children in key order; physical file compaction and exhaustive mutation-write fault\n//! injection remain deferred.\n",
    "//! internal children in key order. Deterministic mutation fault tests inject pre-write, torn-half-\n//! write, and post-sync errors across appended/recycled data pages plus allocation/root superblocks;\n//! physical file compaction and exhaustive device/syscall failure modeling remain deferred.\n",
    "crate docs fault status",
)

replace_once(
    "README.md",
    "Ordered B+ tree scans walk the internal hierarchy in key order rather than persisting leaf sibling\nlinks, so scans introduce no extra COW/reuse edge. Physical file compaction/truncation remains deferred.\n",
    "Ordered B+ tree scans walk the internal hierarchy in key order rather than persisting leaf sibling\nlinks, so scans introduce no extra COW/reuse edge. Deterministic mutation fault tests exercise appended\nand recycled overflow/leaf/internal page writes, allocation metadata, and final root publication under\npre-write, torn-half-write, and post-sync error modes. Reopen must select either the complete old tree or\nthe complete new tree; only a fully synchronized root superblock whose caller still receives an I/O\nerror may expose the new state. Torn recycled orphans remain unreachable and are proven safely\noverwriteable by a later mutation. Physical file compaction/truncation and arbitrary device/cache\npower-loss modeling remain deferred.\n",
    "README B+ tree fault evidence",
)

replace_once(
    "docs/btree-page-format.md",
    "common point-operation `KvEngine` contract, and bounded half-open ordered range scans. Physical file\ncompaction and exhaustive mutation-write fault injection remain deferred. All integers are unsigned\nlittle-endian unless stated otherwise.\n",
    "common point-operation `KvEngine` contract, bounded half-open ordered range scans, and a deterministic\nmutation fault matrix over durable data/metadata write classes. Physical file compaction and exhaustive\ndevice/syscall failure modeling remain deferred. All integers are unsigned little-endian unless stated\notherwise.\n",
    "page format opening",
)

replace_once(
    "docs/btree-page-format.md",
    "This protocol therefore avoids an\nin-place torn-page update problem; it does **not** reclaim unreachable historical pages yet.\n\nThe deterministic test suite also constructs a committed-but-unpublished shadow page and verifies that\nreopen continues to return the value reachable through the older root.\n\n## Reachability-derived page reuse\n",
    "This protocol therefore avoids repairing a torn reachable page in place. Committed pages from an\nunpublished attempt remain outside the authoritative root and become candidates for the same\nreachability-derived reuse mechanism as older COW history.\n\nThe deterministic test suite also constructs a committed-but-unpublished shadow page and verifies that\nreopen continues to return the value reachable through the older root.\n\n## Deterministic mutation fault matrix\n\nThe test-only pager injector records durable write events and can fail one selected event in three\ncontrolled modes: before any bytes are written, after writing and synchronizing half of the 4 KiB image,\nor after writing and synchronizing the complete image but before reporting success to the caller. The\nlast mode models an ambiguous acknowledgement: durable state may have advanced even though the API\nreturns an I/O error and poisons the live handle. Production builds do not carry the injector state.\n\n| Durable write class | Representative coverage | Reopen invariant after injected error |\n| --- | --- | --- |\n| Appended data page | overflow, leaf, internal | old logical root remains authoritative; a partial tail is truncated, while a fully committed unpublished page is merely unreachable |\n| Allocation superblock | page-count publication | a torn copy falls back to the prior mirror; a fully synchronized copy may commit extra unreachable allocation history, but not a new logical root |\n| Recycled data page | overflow, leaf, internal | old root never references the orphan; even a torn recycled image is later proven safe to overwrite completely and reuse |\n| Final root superblock | root install or root clear | before/torn write reopens the old tree; only a complete synchronized root image followed by an injected error may reopen the complete new tree |\n\nThe matrix runs real file writes, `sync_data`, handle poisoning, drop, and reopen. It also covers deleting\nthe final key, where the only logical publication is `root = 0`: pre-write/torn metadata faults preserve\nthe key, while a post-sync reported error reopens the fully empty tree. These tests establish the legal\nsoftware fault states around the engine's durable-write protocol; they are not a claim that every\nfilesystem, controller cache, storage device, or power-loss behavior honors the operating-system sync\ncontract.\n\n## Reachability-derived page reuse\n",
    "fault matrix section",
)

replace_once(
    "docs/design-space.md",
    "| Binary KV + B+ tree + standalone | Common persistent point + ordered-range engine implemented | `db-storage-btree` implements `KvEngine` for the full 4 KiB-key/1 MiB-value point contract plus bounded half-open ordered scans. Scans traverse internal children in separator order without leaf sibling links and are differentially checked against the memory oracle after splits, reopen, delete, and overflow-value materialization; physical compaction remains deferred |\n",
    "| Binary KV + B+ tree + standalone | Common persistent point + ordered-range engine implemented | `db-storage-btree` implements `KvEngine` for the full 4 KiB-key/1 MiB-value point contract plus bounded half-open ordered scans. Scans traverse internal children in separator order without leaf sibling links and are differentially checked against the memory oracle. A deterministic durable-write fault matrix covers appended/recycled overflow, leaf, and internal pages plus allocation/root superblocks with pre-write, torn-half-write, and post-sync errors, proving old-or-complete-new reopen behavior and safe orphan repair; physical compaction remains deferred |\n",
    "design-space B+ tree evidence",
)

replace_once(
    "docs/experimental-constitution.md",
    "Crash consistency is demonstrated through fault injection or prefix/corruption fixtures, never by the\nmere presence of a log or checksum. The B+ tree defines its legal COW/root-publication crash states in\nits page-format specification; exhaustive mutation-write fault injection remains required. Future LSM\nengines must likewise define and test each state transition.\n",
    "Crash consistency is demonstrated through fault injection or prefix/corruption fixtures, never by the\nmere presence of a log or checksum. The B+ tree defines its legal COW/root-publication crash states in\nits page-format specification and exercises a deterministic durable-write matrix: appended/recycled\noverflow, leaf, and internal pages plus allocation/root superblocks are failed before write, after a\nsynchronized half write, and after a complete synchronized write whose acknowledgement is forced to\nfail. Reopen must expose the complete old tree or complete new tree, and the live handle is poisoned\nafter every injected write error. This is a software fault model under the stated sync contract, not an\nexhaustive model of device/controller/power-loss behavior. Future LSM engines must likewise define and\ntest each state transition.\n",
    "constitution B+ tree crash evidence",
)

replace_once(
    "docs/roadmap.md",
    "- [ ] Add torn-page/update fault injection and deterministic crash-state regressions beyond the\n  unpublished-shadow-page protocol before performance work.\n",
    "- [x] Add deterministic mutation fault injection beyond the unpublished-shadow-page protocol. The\n  pager records durable write classes and tests pre-write, synchronized half-write, and post-sync\n  reported failures across appended/recycled overflow, leaf, and internal pages plus allocation/root\n  superblocks. Reopen is constrained to the complete old or complete new tree; tests also cover final\n  root clearing and prove torn recycled orphans can be overwritten safely by a later mutation.\n",
    "Phase 2 fault checkbox",
)
