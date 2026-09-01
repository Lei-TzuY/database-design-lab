# Append-log generation-aware workload runner

`db-lab-log-generation-run` executes a normal versioned `Workload` through `GenerationLogEngine` rather than opening a generation log by raw pathname.

```console
cargo run -p db-cli --bin db-lab-log-generation-run -- \
  --directory generations \
  --workload workload.json
```

The runner is intended for correctness experiments and regression workloads after legacy migration/cutover or after a generation directory already exists. It acquires the same cooperative generation-writer lease as other generation-aware mutations, refreshes authority before each workload operation, and follows a newer committed generation if authority advances between operations.

The JSON result reports the engine capability name, workload format version and seed, executed step count, final authoritative generation/path, and every observable workload outcome.

The workload input is bounded to 64 MiB, must be a real regular file, must deserialize as the repository's versioned `Workload`, and must pass common operation validation before any generation handle is opened.

This command is the supported workload-level mutation path for generation directories. The mutating raw CLI paths `db-lab run --engine log` and `db-lab differential --engine log` reserve strict canonical `generation-{id:020}.log` names and fail before opening or creating such a pathname, directing callers here instead. This guard is intentionally mutation-only: `db-lab verify` and `db-lab inspect` may still read canonical generation files directly as evidence/diagnostic inputs.

The raw single-file laboratory engine remains supported under ordinary noncanonical filenames. Direct library users with arbitrary filesystem access are still outside the cooperative protocol's threat model; the generation layer and CLI guard provide correctness coordination and misuse resistance, not an OS sandbox.
