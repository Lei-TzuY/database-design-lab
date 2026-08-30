# Controlled publication sessions

`db-lab-publication-session` binds one verified, passing `linux_controlled_host_preflight_v1` snapshot to one verified publication-admitted repeated-batch archive without mutating the existing v7/v11 raw evidence schemas.

The session protocol is `controlled_publication_session_v1`, format version 1. It is an evidence-binding artifact, not a benchmark result, cryptographic signature, proof of operator truthfulness, or statistical regression gate.

## Why a separate artifact exists

The repeated-batch archive already has its own immutable versioned contract and `publication_warm_v1` admission. The host preflight has a separate versioned contract because machine-observable affinity/frequency/load controls are different evidence from the benchmark trace and outcomes. Silently adding preflight data to v7/v11 would redefine already-frozen formats.

A publication session therefore carries exact copies of both inputs and records their binding in a new outer index.

## Session layout

A v1 session contains exactly:

```text
session/
  index.json
  host-preflight.json
  evidence/
    index.json
    trace.json
    batch.json
    environment.json
    [comparison-failures.json for v11]
```

`index.json` records:

- session format and protocol;
- required host-preflight protocol;
- required publication admission protocol;
- the exact host label shared by both inputs;
- host-preflight recording time;
- repository revision;
- source repeated-archive format version;
- the exact sorted evidence file set.

Only publication repeated formats v7 and v11 are admissible.

## Create

```text
db-lab-publication-session create \
  --host-preflight evidence/host-preflight-001.json \
  --archive-dir evidence/batch-publication-001 \
  --session-dir evidence/session-001 \
  --expected-revision 0123456789abcdef0123456789abcdef01234567
```

Creation is fail closed:

1. the source preflight must be a regular file and pass the shared host-preflight verifier with `require_passed=true`;
2. the source archive must be a real directory and pass the shared batch verifier with publication required;
3. `environment.json.publication_admission.host_label` must equal the verified preflight host label;
4. the destination must not already exist and must not overlap the source archive;
5. both inputs are copied with create-new writes into the fresh session;
6. the copies are re-verified using the same shared verifiers and the host/revision/format binding is checked again;
7. the sources are re-verified after copying and compared byte-for-byte with the retained copies;
8. only then is the session `index.json` written;
9. the completed session is immediately verified through the public CLI path.

Any create failure removes the partially created session directory.

## Verify

```text
db-lab-publication-session verify \
  --session-dir evidence/session-001 \
  --expected-revision 0123456789abcdef0123456789abcdef01234567
```

Verification requires the exact top-level layout, a strict versioned index, a passing retained host-preflight snapshot, a publication-admitted retained batch archive, the indexed evidence file set, the expected repository revision when supplied, the indexed v7/v11 source format, and one identical host label across the preflight, session index, and archive publication admission.

Changing only the preflight host label or only `environment.json.publication_admission.host_label` therefore invalidates the session even when each JSON document remains syntactically valid. The inner batch verifier remains responsible for the complete repeated-evidence contract; the session layer only adds the cross-artifact binding.

## CI and synthetic publication fixtures

Repository CI may exercise the session create/verify logic using verifier-compatible synthetic v7/v11 fixtures. That proves artifact semantics and portability only. It does not turn GitHub-hosted runners into publication performance hosts and does not make synthetic timing evidence publishable.

A real publication session still requires a real named Linux performance host, an actually passing preflight captured in the collection environment, a real `publication_warm_v1` repeated archive from that host, and subsequent human review of the denominator and descriptive distributions.

## Phase 4 boundary

With this protocol, the repository can preserve raw repeated evidence, verify it, analyze it from a byte-stable snapshot, package the analysis beside exact raw bytes, collect and re-verify controlled-host preflight evidence, and bind a passing preflight to publication-admitted raw evidence without redefining old formats.

The controlled-host roadmap item remains incomplete until a real host is configured and actual controlled measurements are collected. Regression thresholds remain later work after reviewed real distributions exist.
