# LSM SSTable, manifest, and WAL publication v2

This document specifies persistent sorted-table publication plus crash-safe WAL segmentation and
reclamation. Mutations are acknowledged by a checksummed WAL segment before entering memory. A frozen
MemTable is installed as an immutable indexed SSTable through an immutable manifest snapshot and mirrored
`CURRENT`; when that durable watermark reaches the active WAL tail, Manifest v2 can atomically switch the
version set to a new empty WAL and reclaim older segments only after both CURRENT mirrors move. Bloom
filters, levels, compaction, and tombstone dropping remain outside this version.

All integers are unsigned little-endian. The common 4,096-byte key and 1,048,576-byte value limits
remain authoritative. SSTables keep sequence numbers and explicit tombstones so reads can resolve
newer WAL/MemTable state without discarding deletion history prematurely.

## Directory state

A newly created engine contains:

```text
wal-0000000000000001.log
CURRENT
MANIFEST-0000000000000001
```

Published flushes add canonical immutable files:

```text
sst-0000000000000001.sst
MANIFEST-0000000000000002
sst-0000000000000002.sst
MANIFEST-0000000000000003
...
```

`CURRENT` determines the authoritative manifest. That manifest determines the authoritative WAL segment.
Canonically named SSTables, manifests, or WALs not selected by this chain are orphans from interrupted or
superseded publication and are not opened as authoritative state. Their ids still reserve numeric space,
so later allocation never overwrites a file whose durability outcome was ambiguous. Unknown names and
non-regular entries fail closed.

For compatibility with the earlier WAL/MemTable Phase 3 slice, an existing directory containing
exactly the canonical WAL and no version-set files opens as a legacy layout with durable sequence zero.
It is replayed completely. The first later flush creates the initial manifest/CURRENT state before
installing its SSTable. A partial version-set layout is never treated as legacy: CURRENT without a
manifest, manifests without CURRENT, or SSTables in an otherwise WAL-only directory fail closed.

As with the existing formats, file contents are synchronized but this implementation does not claim a
portable parent-directory fsync protocol. The experimental constitution's directory-entry durability
caveat therefore still applies to sudden system loss immediately after file creation.

## SSTable v1

Each `sst-%016d.sst` is immutable once created. It consists of a 64-byte header, strictly sorted data
records, a complete sorted index, and a 64-byte footer. The file is written with `create_new`, then
`write_all`, then `sync_all` before any manifest may reference it.

The header contains magic `DBLSMSST`, format version 1, table id, entry count, data/index/footer offsets,
reserved zero bytes, and a header CRC-32. Each data record contains magic `SSTR`, record version, PUT or
DELETE kind, sequence, bounded key/value lengths, header CRC, key/value bytes, and a record CRC. DELETE
records carry zero value bytes. Keys must be strictly increasing.

The index contains one entry for every data record: full key, record kind, sequence, physical data-record
offset, and checksum. Opening validates that every index entry exactly describes the corresponding data
record and that both sections have identical strictly increasing keys. This first implementation keeps
the validated file bytes and full index resident in memory; that is a correctness-first choice and is
not yet evidence for realistic read amplification.

The footer repeats table id, entry count, index/footer offsets, and the table's durable WAL sequence.
It includes a CRC of every byte before the footer plus its own checksum. The manifest descriptor also
records physical file length, entry count, durable sequence, and smallest/largest key. Open requires all
three views—header/footer/manifest—to agree.

## Immutable manifest snapshots

A `MANIFEST-%016d` is a complete version-set snapshot, not an append log. Manifest v1 uses its original
64-byte header and remains readable with implicit WAL id 1 / first sequence 1. Newly written Manifest v2
uses an 80-byte header that stores manifest id, durable sequence, table count, descriptor-body length,
a nonzero authoritative WAL id, and that WAL's nonzero first sequence, followed by reserved zeros and a
header CRC. Every SSTable descriptor is individually checksummed, and the entire manifest has a trailing
CRC.

Descriptors are ordered by strictly increasing table id and durable sequence. The manifest-level durable
sequence must equal the newest descriptor's watermark, or zero when the table set is empty. No manifest
is accepted merely because the WAL could reconstruct its data: a manifest selected by CURRENT is part
of authoritative persistent state, so checksum or descriptor corruption fails closed.

## Mirrored CURRENT publication

`CURRENT` is exactly 8,192 bytes: two independent 4 KiB slots. Each slot contains magic `DBLSMCUR`,
version 1, its physical slot id, generation, manifest id, reserved zeros, and a slot CRC. Both initial
slots name MANIFEST 1 at generation zero.

For each version-set install, generation increases by one and the inactive slot (`generation % 2`) is
overwritten and synchronized. If one slot is invalid, the other is sufficient. Two valid equal-generation
slots must agree on manifest id; two unequal valid generations must differ by exactly one. The newer
valid slot wins.

## Flush/install ordering

A frozen MemTable is published synchronously in this order:

1. The mutation records that formed the frozen table are already complete and synchronized in the WAL.
2. Create the next immutable SSTable under its final canonical id, write all bytes, and `sync_all`.
3. Create a new immutable manifest snapshot containing all previously published tables plus the new
   descriptor, write all bytes, and `sync_all`.
4. Overwrite the inactive CURRENT slot with generation + 1 and the new manifest id, then `sync_data`.
5. Only after step 4 succeeds does the live engine retire the frozen MemTable and expose the new
   version set as authoritative.

A failure during any flush/install step poisons the live engine because the caller cannot safely infer
which durable writes completed. Reopen is the recovery boundary.

## Legal interrupted-install states

If interruption occurs before CURRENT publication, the older CURRENT/manifest remains authoritative.
A complete or partial new SSTable/manifest is not referenced by that version set. Because the WAL still
retains the complete mutation history, reopen replays every sequence above the older manifest's durable
watermark and reconstructs the missing logical state.

If the new CURRENT slot is torn, its CRC fails and the prior mirrored slot wins; WAL replay again fills
the unpublished suffix. If the new CURRENT slot is fully durable, every SSTable and manifest it names
was synchronized first, so reopen may safely skip WAL sequences through the new durable watermark.

The deterministic recovery tests exercise a damaged latest CURRENT slot, referenced SSTable corruption,
referenced manifest corruption, canonical unreferenced orphans, published SSTable plus mutable WAL tail,
and 1 MiB value flush/reopen. This is a software/process interruption model under successful OS sync
ordering, not a claim about hardware that violates that contract.

## WAL rotation, mirror safety, and reads

Rotation is eligible only when the active WAL contains at least one record and its `next_sequence` is
exactly `durable_sequence + 1`; this proves no unflushed mutable suffix still depends on that segment. The
engine then creates and synchronizes a new empty canonical WAL whose first sequence is
`durable_sequence + 1`, writes/synchronizes a new Manifest v2 naming the new WAL and unchanged SSTable
set, and publishes that manifest through the next CURRENT generation.

At this point the older CURRENT mirror may still select the previous manifest/WAL, so the old WAL is not
yet reclaimable. The engine writes the **same new manifest id** into the other CURRENT slot at the next
generation and synchronizes it. Only after both valid mirrors select the new manifest does it swap/drop
the old WAL handle and best-effort remove every other canonical WAL segment. This ordering is also
Windows-safe because no obsolete WAL is removed while its live file handle is retained.

A crash before the first CURRENT update leaves the old manifest/WAL authoritative and the new files as
canonical orphans. A crash after the first CURRENT update but before mirror completion can select either
version, so both WALs remain present. After mirror completion, either CURRENT slot selects the same new
manifest/WAL and old segments are redundant. A replayed frozen MemTable followed by a newer mutable tail
specifically cannot trigger reclamation until later flushes advance the durable watermark through that
tail.

Point reads search mutable/frozen MemTables first and then authoritative SSTables newest-first. Ordered
range scans merge sequence-tagged SSTable state and the active WAL/MemTable tail, keep the newest version
of each key, remove tombstones, and apply the common half-open bounds/limit. Compaction is still required
before old SSTable versions or tombstones can be discarded safely.

## Explicitly deferred

Bloom filters, leveled placement, overlap rules, compaction selection, crash-safe compaction publication,
obsolete SSTable/manifest deletion, tombstone elision, block/cache design, and amplification
instrumentation remain separate evidence milestones. WAL segment rotation/reclamation is now part of the
implemented crash-state protocol; parent-directory durability remains subject to the repository-wide
portable-fsync caveat.
