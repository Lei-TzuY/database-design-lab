# LSM WAL and MemTable foundation v1

This document specifies only the Phase 3 behavior implemented today: one checksummed write-ahead log,
one ordered mutable MemTable, and zero or more ordered immutable MemTables reconstructed in memory.
There are no SSTables, manifests, Bloom filters, levels, compaction, background flushes, or WAL
reclamation yet. Consequently this engine is executable correctness evidence, but not yet a candidate
for B+ tree versus LSM performance claims.

All encoded integers are unsigned little-endian. Keys and values follow the common 4,096-byte and
1,048,576-byte limits. Empty keys and PUT values are valid; a DELETE is distinguished by record kind,
not by an empty value.

## Directory layout

An engine path is a directory containing exactly one regular file:

```text
wal-0000000000000001.log
```

Opening an existing empty directory, an unknown entry, a non-regular WAL entry, or a missing WAL fails
closed. `create_new` atomically reserves the directory name and rejects every existing path. If initial
WAL creation fails, the reserved but incomplete directory remains invalid rather than being guessed at
or silently reinitialized. The implementation synchronizes the initial WAL contents, but does not use a
platform-specific parent-directory sync; a system crash immediately after creation may therefore lose
the directory entry.

The fixed file name and WAL id deliberately leave no implied segment-rotation protocol. WAL numbering,
SSTable naming, temporary-file handling, and manifest publication will be specified together when real
flush/install behavior exists.

## WAL header

The active WAL begins with a 40-byte header. The CRC is IEEE CRC-32.

| Offset | Bytes | Meaning | v1 validation |
| ---: | ---: | --- | --- |
| 0 | 8 | magic `DBLSMWAL` | exact match |
| 8 | 2 | format version | `1` |
| 10 | 2 | header length | `40` |
| 12 | 4 | flags | zero |
| 16 | 8 | WAL id | `1`, matching the fixed file name |
| 24 | 8 | first sequence | `1` |
| 32 | 4 | reserved | zero |
| 36 | 4 | header CRC-32 | checksum of bytes 0–35 |

Unknown versions, flags, ids, nonzero reserved fields, bad checksums, and truncated headers are rejected.

## Mutation record

Each complete mutation consists of a 32-byte header followed immediately by `key || value`. DELETE has
no value bytes. Sequence numbers are contiguous and start at one.

| Offset | Bytes | Meaning | v1 validation |
| ---: | ---: | --- | --- |
| 0 | 4 | magic `LSMR` | exact match |
| 4 | 1 | record version | `1` |
| 5 | 1 | kind | `1` PUT, `2` DELETE |
| 6 | 2 | flags | zero |
| 8 | 8 | sequence | exactly the next expected sequence |
| 16 | 4 | key length | at most 4,096 |
| 20 | 4 | value length | at most 1,048,576; zero for DELETE |
| 24 | 4 | header CRC-32 | checksum of bytes 0–23 |
| 28 | 4 | record CRC-32 | checksum of bytes 0–27 plus key/value payload |
| 32 | variable | key then value | exact validated lengths |

Decoding validates the fixed header and bounded lengths before allocating the payload. Every addition
used to derive payload size or physical record end uses checked arithmetic. A complete checksum failure,
unknown kind/version/flag, impossible length, or sequence discontinuity is corruption even at EOF.

## Commit and recovery rule

A mutation is acknowledged in this order:

1. validate the common key/value bounds;
2. encode the next contiguous WAL record;
3. seek to WAL EOF, `write_all` the record, and call `sync_data`;
4. apply the same sequence/key/value or tombstone to the mutable MemTable; and
5. return the previous logical value.

An I/O error makes the outcome ambiguous and poisons the live engine until reopen. A complete record
may replay even if its caller observed an error; a partial final record does not replay.

Read-only `verify` reports but does not repair a final incomplete record. Mutating `open` truncates back
to the previous complete record boundary and calls `sync_all` only when the bytes present are a canonical
prefix of the next expected record: magic, version, kind, flags, sequence, any available bounded lengths,
and a complete available header checksum must all agree. Once a complete header is present, it also
reports the required full record size. Unrecognized trailing bytes and complete checksum failures fail
closed rather than being discarded.

The deterministic tests cut the final record at magic, version, kind, flags, sequence, length, checksum,
and payload boundaries; they also exercise bit flips, absurd declared lengths, unexplained tails, and
sequence gaps. This is a process-interruption byte-prefix model under the operating system's sync
contract, not evidence about devices or controller caches that violate successful sync ordering.

## MemTable state

Each MemTable is an ordered map from binary key to `(sequence, value-or-tombstone)`. Point reads search
the mutable table and then immutable tables newest-first. Ordered scans merge the sequence-tagged table
contents into the newest visible version of every key, remove tombstones, and return the common bounded
half-open bytewise range.

After each replayed or live mutation, a mutable table whose deterministic resident estimate reaches
64 KiB is frozen into the immutable list and replaced by an empty mutable table. The estimate is
`24 + key length + value length` bytes per latest entry in that table; replacing a key subtracts its old
estimate first. This is a structure threshold, not a measured allocator byte count or a published memory
amplification metric. Replaying the ordered WAL with the same fixed threshold reconstructs the same
boundaries.

Immutable here means no later mutation changes that in-memory table. It does **not** mean persisted:
every table still depends on the unreclaimed WAL after restart. There is no background worker or
concurrent mutation contract; one caller and one process must serialize access.

## Explicitly deferred

The following require another focused design plus executable evidence: immutable SSTable bytes and
indexes, block checksums, manifest/CURRENT formats, atomic version-set installation, flush crash states,
WAL rotation/reclamation, Bloom filters, levels, tombstone dropping rules, compaction, read/write/space
amplification instrumentation, and performance comparisons. Unknown directory entries are rejected
precisely because none of those formats has been declared yet.
