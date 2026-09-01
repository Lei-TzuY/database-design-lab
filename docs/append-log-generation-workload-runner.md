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

This command is the supported workload-level mutation path for generation directories. `db-lab run --engine log` remains the deliberately raw single-file laboratory engine and must not be pointed at canonical generation files. Direct library users with arbitrary filesystem access are still outside the cooperative protocol's threat model; the generation layer is correctness coordination, not an OS sandbox.
