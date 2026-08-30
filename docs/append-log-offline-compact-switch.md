# Offline append-log generation compact switch

`db-lab-log-generation-compact-switch` composes the shared generation-directory verifier, append-log compact-copy implementation, and durable generation-marker publisher into one **offline** authoritative switch on Unix.

```text
db-lab-log-generation-compact-switch \
  --directory data/log-generations
```

The protocol is `append_log_offline_generation_compact_switch_unix_v1`.

## Required operating condition

Every raw-path writer that can mutate an append-log generation directly must remain quiesced for the entire command. This is a real correctness precondition, not a performance recommendation.

`GenerationLogEngine` writers use the same cooperative sibling lease as the switch. They may run while the expensive compact copy is built; the switch then acquires the lease, detects any source drift, and fails before marker publication. They cannot run during the final recheck-to-publication critical section. The repository still exposes raw-path `LogEngine`, however, and such callers bypass this lease. The operation therefore remains an offline maintenance primitive rather than claiming protection from non-cooperating writers.

On non-Unix targets the operation returns unsupported before touching the supplied directory. In particular, Windows remains disabled because the repository does not yet claim an equivalent durable parent-directory publication primitive there.

## Successful order

The switch preserves the frozen generation recovery law:

1. verify `append_log_generation_directory_v2` and select only the highest final committed marker;
2. snapshot the authoritative generation's complete inspected live state and marker/recovery witness;
3. allocate the next generation id strictly above **every** observed generation log, final marker, and staging marker id;
4. build a fresh compact log at that canonical generation path through `append_log_compact_copy_v1`;
5. acquire the cooperative writer lease, re-verify the generation directory, and re-inspect the old authoritative source;
6. require the authoritative generation, committed-prefix witness, complete log verification, and complete live state to be unchanged;
7. independently inspect the compact target and require exact live-state equality;
8. publish its marker through `append_log_generation_marker_publication_unix_v1`;
9. re-run generation-directory verification and require the new generation to be authoritative;
10. re-inspect the new authoritative generation and require exact equality with the pre-switch live state.

The old generation and old marker are retained. This command performs no historical cleanup.

## Race and crash behavior

A new compact generation has no authority until its final marker is durably published. If the old authoritative source changes before the pre-publication recheck, the command fails with `SourceChanged`; the compact target remains only an uncommitted orphan and recovery continues to select the old committed generation.

The test suite injects a deterministic late source write after compact construction and before marker publication. It requires the switch to fail, requires the new generation file to remain uncommitted, and requires the v2 reader to keep the old generation authoritative.

The allocation frontier also treats uncommitted generation logs and staging markers as consumed ids. Crash residue is never silently reused: if the highest observed id is 7, the next switch attempts generation 8 even when only generation 1 is currently committed.

Marker publication retains its own durability-uncertain state. If the final marker becomes visible but the final parent-directory durability barrier fails, the operation returns the publication error and does not pretend the switch failed before authority could have changed. Operators must preserve retained generations and run the generation verifier before deciding how to retry.

## Deterministic composed interruption matrix

The unit-test fault harness injects one reported failure at each retained-state boundary in the composed switch. Each case starts from the same deterministic binary-KV history, drops the live operation, runs the normal generation-directory verifier, and requires the selected generation to contain exactly the source live entries.

| Injected boundary | Retained evidence | Permitted recovery |
| --- | --- | --- |
| compact candidate published | visible complete uncommitted generation; directory entry not yet synchronized | old only |
| lease-held source/candidate recheck complete | visible complete uncommitted generation; directory entry not yet synchronized | old only |
| generation file and directory synchronized | durable uncommitted generation | old only |
| half of staging marker written | truncated non-authoritative staging marker | old only |
| staging marker synchronized | complete non-authoritative staging marker | old only |
| final marker hard-linked, before directory sync | final marker may survive or be lost; staging remains | old or new |
| final marker directory sync complete | durable final marker; staging may remain | new only |
| publisher complete | durable final marker; staging cleanup attempted | new only |

For the pre-directory-sync hard-link boundary, the ordinary injected-error fixture observes the visible final marker and selects new. A second deterministic fixture models the other filesystem-permitted outcome by removing that unsynchronized directory entry and requires recovery to select old. Both generations contain the same logical state; the test does not claim that deleting an entry is a physical power-loss emulator.

Injected errors unwind normally, so a lease acquired by the operation is released. A real process death may retain the sibling writer-lock evidence and must use the guarded stale-lock recovery procedure. Hosted CI establishes protocol and recovery behavior for these concrete namespace states; it is not evidence that every filesystem/hardware stack implements power-loss durability identically.

## What this completes

This is the first concrete operation in the repository that performs the complete old-authority -> fresh compact generation -> durable marker -> new-authority sequence without subprocess composition or duplicated recovery rules.

It closes these previously separate repository-side pieces for the Unix offline case:

- authoritative source selection;
- fresh allocation above crash residue;
- exact live-state compaction;
- stale-source detection before publication;
- durable marker publication;
- deterministic fault coverage across every composed retained-state boundary;
- exact old-or-new logical recovery through the normal generation verifier;
- post-publication authoritative/state verification.

## What remains

The broader Phase 1 compaction milestone remains open because the append-log engine still lacks several lifecycle pieces:

- Windows-equivalent durable marker publication;
- safe deletion policy for obsolete committed generations and uncommitted crash orphans;
- migration/coexistence rules for legacy one-file append-log users;
- stronger ownership if non-cooperating raw-path writers must be prevented rather than administratively quiesced.

Until those exist, this command is an explicit offline maintenance primitive, not a transparent production compactor.
