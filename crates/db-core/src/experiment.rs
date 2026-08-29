use serde::{Deserialize, Serialize};

use crate::{
    validate_experiment_compatibility, validate_key, validate_key_value, validate_range_scan,
    AmplificationInstrumented, AmplificationRatio, AmplificationReport, ByteString, DbError,
    EngineCapabilities, KvEngine, ReadWorkUnit, Result, StructuralReadAmplification,
    MAX_VALUE_BYTES, MAX_WORKLOAD_STEPS,
};

/// JSON schema version for Phase 4 experiment traces.
pub const EXPERIMENT_TRACE_FORMAT_VERSION: u16 = 1;
/// Stable generator revision recorded in every generated trace.
pub const EXPERIMENT_GENERATOR_REVISION: u16 = 1;
/// Defensive upper bound for generated key-space cardinality.
pub const MAX_EXPERIMENT_KEY_SPACE: u32 = 1_000_000;
/// Defensive upper bound for one generated range-scan limit.
pub const MAX_EXPERIMENT_RANGE_WIDTH: u32 = 1_000_000;

/// Reproducible operation family used by the Phase 4 storage comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentProfile {
    /// Preload a common key set, then issue random successful/missing point reads.
    PointRead,
    /// Preload a common ordered key set, then issue bounded half-open range scans.
    RangeScan,
    /// Issue puts over a cyclic ascending key sequence.
    SequentialWrite,
    /// Issue puts over a seeded random key sequence.
    RandomWrite,
    /// Preload a common key set, then mix puts, gets, deletes, and range scans.
    Mixed,
}

/// Inputs that completely determine one generated experiment trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentConfig {
    /// SplitMix64 seed. The profile is mixed into the generator stream independently.
    pub seed: u64,
    /// Number of measured logical operations, excluding inserted reopen actions.
    pub operations: u32,
    /// Cardinality of the fixed-width ordered key domain.
    pub key_space: u32,
    /// Exact generated value size for puts.
    pub value_bytes: u32,
    /// Width and result limit used by generated range scans.
    pub range_width: u32,
    /// Insert a reopen after this many measured logical operations.
    pub reopen_every: Option<u32>,
}

/// One logical action in a reproducible storage experiment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ExperimentStep {
    /// Set one opaque binary key/value pair.
    Put { key: ByteString, value: ByteString },
    /// Read one opaque binary key.
    Get { key: ByteString },
    /// Delete one opaque binary key.
    Delete { key: ByteString },
    /// Read at most `limit` records from the half-open ordered interval `[start, end)`.
    RangeScan {
        start: ByteString,
        end: Option<ByteString>,
        limit: u32,
    },
    /// Close/reopen persistent state without resetting the measurement window.
    Reopen,
}

/// Versioned trace split into setup and measured phases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentTrace {
    /// Trace schema version.
    pub format_version: u16,
    /// Stable generator implementation revision.
    pub generator_revision: u16,
    /// Operation family represented by this generated trace.
    pub profile: ExperimentProfile,
    /// Complete generator configuration.
    pub config: ExperimentConfig,
    /// Deterministic state preparation excluded from amplification counters.
    pub setup: Vec<ExperimentStep>,
    /// Deterministic operations included in amplification counters.
    pub measured: Vec<ExperimentStep>,
}

/// Observable logical result of one experiment action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ExperimentOutcome {
    /// Previous value returned by PUT.
    Put { previous: Option<ByteString> },
    /// Value returned by GET.
    Get { value: Option<ByteString> },
    /// Previous value returned by DELETE.
    Delete { previous: Option<ByteString> },
    /// Ordered rows returned by a range scan.
    RangeScan { rows: Vec<(ByteString, ByteString)> },
    /// Reopen completed successfully.
    Reopened,
}

/// One engine's raw structural/data-path evidence for a common trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EngineAmplificationEvidence {
    /// Exact capabilities used by experiment preflight.
    pub capabilities: EngineCapabilities,
    /// Exact integer amplification report for the measured window.
    pub amplification: AmplificationReport,
}

/// Reproducible cross-engine evidence produced only after step-by-step logical agreement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExperimentComparisonReport {
    /// Trace schema version.
    pub trace_format_version: u16,
    /// Stable generator revision.
    pub generator_revision: u16,
    /// Operation family.
    pub profile: ExperimentProfile,
    /// Complete generator configuration.
    pub config: ExperimentConfig,
    /// FNV-1a fingerprint of the exact serialized trace input.
    pub trace_fingerprint: String,
    /// Number of setup actions checked on both engines.
    pub setup_steps_checked: usize,
    /// Number of measured actions checked on both engines.
    pub measured_steps_checked: usize,
    /// Fingerprint of the common setup outcomes after both engines agreed step by step.
    pub setup_outcome_fingerprint: String,
    /// Fingerprint of the common measured outcomes after both engines agreed step by step.
    pub measured_outcome_fingerprint: String,
    /// Left-hand engine evidence.
    pub left: EngineAmplificationEvidence,
    /// Right-hand engine evidence.
    pub right: EngineAmplificationEvidence,
}

impl ExperimentConfig {
    fn validate(self) -> Result<()> {
        if self.operations == 0 {
            return Err(DbError::InvalidInput(
                "experiment operations must be greater than zero".to_owned(),
            ));
        }
        if self.key_space == 0 {
            return Err(DbError::InvalidInput(
                "experiment key_space must be greater than zero".to_owned(),
            ));
        }
        if self.key_space > MAX_EXPERIMENT_KEY_SPACE {
            return Err(DbError::InvalidInput(format!(
                "experiment key_space is {}; maximum is {MAX_EXPERIMENT_KEY_SPACE}",
                self.key_space
            )));
        }
        if usize::try_from(self.value_bytes).unwrap_or(usize::MAX) > MAX_VALUE_BYTES {
            return Err(DbError::InvalidInput(format!(
                "experiment value_bytes is {}; maximum is {MAX_VALUE_BYTES}",
                self.value_bytes
            )));
        }
        if self.range_width == 0 {
            return Err(DbError::InvalidInput(
                "experiment range_width must be greater than zero".to_owned(),
            ));
        }
        if self.range_width > self.key_space || self.range_width > MAX_EXPERIMENT_RANGE_WIDTH {
            return Err(DbError::InvalidInput(format!(
                "experiment range_width is {}; it must not exceed key_space {} or maximum {MAX_EXPERIMENT_RANGE_WIDTH}",
                self.range_width, self.key_space
            )));
        }
        if self.reopen_every == Some(0) {
            return Err(DbError::InvalidInput(
                "experiment reopen_every must be greater than zero".to_owned(),
            ));
        }
        Ok(())
    }
}

impl ExperimentTrace {
    /// Validates versioning, configured bounds, total step count, and every encoded operation.
    pub fn validate(&self) -> Result<()> {
        if self.format_version != EXPERIMENT_TRACE_FORMAT_VERSION {
            return Err(DbError::UnsupportedVersion {
                format: "experiment trace",
                found: u64::from(self.format_version),
                supported: u64::from(EXPERIMENT_TRACE_FORMAT_VERSION),
            });
        }
        if self.generator_revision != EXPERIMENT_GENERATOR_REVISION {
            return Err(DbError::UnsupportedVersion {
                format: "experiment trace generator revision",
                found: u64::from(self.generator_revision),
                supported: u64::from(EXPERIMENT_GENERATOR_REVISION),
            });
        }
        self.config.validate()?;
        let total = self
            .setup
            .len()
            .checked_add(self.measured.len())
            .ok_or_else(|| DbError::InvalidInput("experiment step count overflowed".to_owned()))?;
        if total > MAX_WORKLOAD_STEPS {
            return Err(DbError::InvalidInput(format!(
                "experiment has {total} steps; maximum is {MAX_WORKLOAD_STEPS}"
            )));
        }
        if self.measured.is_empty() {
            return Err(DbError::InvalidInput(
                "experiment measured phase must not be empty".to_owned(),
            ));
        }
        for step in self.setup.iter().chain(&self.measured) {
            validate_experiment_step(step)?;
        }
        Ok(())
    }

    fn requires_ordered_range(&self) -> bool {
        self.setup
            .iter()
            .chain(&self.measured)
            .any(|step| matches!(step, ExperimentStep::RangeScan { .. }))
    }
}

/// Generates one stable setup/measurement trace from the declared profile and configuration.
pub fn generate_experiment_trace(
    profile: ExperimentProfile,
    config: ExperimentConfig,
) -> Result<ExperimentTrace> {
    config.validate()?;

    let setup_logical = if profile_needs_seeded_state(profile) {
        u64::from(config.key_space) + 1
    } else {
        0
    };
    let reopen_count = config
        .reopen_every
        .map_or(0_u64, |every| u64::from(config.operations / every));
    let total = setup_logical
        .checked_add(u64::from(config.operations))
        .and_then(|count| count.checked_add(reopen_count))
        .ok_or_else(|| DbError::InvalidInput("generated experiment is too large".to_owned()))?;
    let total = usize::try_from(total)
        .map_err(|_| DbError::InvalidInput("generated experiment is too large".to_owned()))?;
    if total > MAX_WORKLOAD_STEPS {
        return Err(DbError::InvalidInput(format!(
            "generated experiment would have {total} steps; maximum is {MAX_WORKLOAD_STEPS}"
        )));
    }

    let mut setup = Vec::with_capacity(usize::try_from(setup_logical).unwrap_or(0));
    if profile_needs_seeded_state(profile) {
        for key_id in 0..config.key_space {
            setup.push(put_step(
                config.seed,
                u64::from(key_id),
                0,
                config.value_bytes,
            )?);
        }
        setup.push(ExperimentStep::Reopen);
    }

    let mut random = SplitMix64::new(config.seed ^ profile_salt(profile));
    let measured_capacity = usize::try_from(
        u64::from(config.operations)
            .checked_add(reopen_count)
            .ok_or_else(|| DbError::InvalidInput("generated experiment is too large".to_owned()))?,
    )
    .map_err(|_| DbError::InvalidInput("generated experiment is too large".to_owned()))?;
    let mut measured = Vec::with_capacity(measured_capacity);
    for operation_index in 0..config.operations {
        let step = match profile {
            ExperimentProfile::PointRead => ExperimentStep::Get {
                key: ByteString::from(experiment_key(random.bounded(u64::from(config.key_space)))),
            },
            ExperimentProfile::RangeScan => range_step(&mut random, config),
            ExperimentProfile::SequentialWrite => put_step(
                config.seed,
                u64::from(operation_index % config.key_space),
                u64::from(operation_index) + 1,
                config.value_bytes,
            )?,
            ExperimentProfile::RandomWrite => put_step(
                config.seed,
                random.bounded(u64::from(config.key_space)),
                u64::from(operation_index) + 1,
                config.value_bytes,
            )?,
            ExperimentProfile::Mixed => mixed_step(&mut random, config, operation_index)?,
        };
        measured.push(step);
        if config
            .reopen_every
            .is_some_and(|every| (operation_index + 1) % every == 0)
        {
            measured.push(ExperimentStep::Reopen);
        }
    }

    let trace = ExperimentTrace {
        format_version: EXPERIMENT_TRACE_FORMAT_VERSION,
        generator_revision: EXPERIMENT_GENERATOR_REVISION,
        profile,
        config,
        setup,
        measured,
    };
    trace.validate()?;
    Ok(trace)
}

/// Executes an identical trace against two instrumented engines and refuses logical divergence.
pub fn run_amplification_comparison<L, R>(
    left: &mut L,
    right: &mut R,
    trace: &ExperimentTrace,
) -> Result<ExperimentComparisonReport>
where
    L: KvEngine + AmplificationInstrumented,
    R: KvEngine + AmplificationInstrumented,
{
    trace.validate()?;
    let left_capabilities = left.capabilities();
    let right_capabilities = right.capabilities();
    validate_experiment_compatibility(
        left_capabilities,
        right_capabilities,
        trace.requires_ordered_range(),
    )?;

    let setup_outcome_fingerprint = execute_matching_phase(left, right, "setup", &trace.setup)?;
    left.reset_amplification();
    right.reset_amplification();
    let measured_outcome_fingerprint =
        execute_matching_phase(left, right, "measured", &trace.measured)?;

    Ok(ExperimentComparisonReport {
        trace_format_version: trace.format_version,
        generator_revision: trace.generator_revision,
        profile: trace.profile,
        config: trace.config,
        trace_fingerprint: fingerprint_serializable(trace)?,
        setup_steps_checked: trace.setup.len(),
        measured_steps_checked: trace.measured.len(),
        setup_outcome_fingerprint,
        measured_outcome_fingerprint,
        left: EngineAmplificationEvidence {
            capabilities: left_capabilities,
            amplification: left.amplification_report()?,
        },
        right: EngineAmplificationEvidence {
            capabilities: right_capabilities,
            amplification: right.amplification_report()?,
        },
    })
}

fn profile_needs_seeded_state(profile: ExperimentProfile) -> bool {
    matches!(
        profile,
        ExperimentProfile::PointRead | ExperimentProfile::RangeScan | ExperimentProfile::Mixed
    )
}

const fn profile_salt(profile: ExperimentProfile) -> u64 {
    match profile {
        ExperimentProfile::PointRead => 0x243f_6a88_85a3_08d3,
        ExperimentProfile::RangeScan => 0x1319_8a2e_0370_7344,
        ExperimentProfile::SequentialWrite => 0xa409_3822_299f_31d0,
        ExperimentProfile::RandomWrite => 0x082e_fa98_ec4e_6c89,
        ExperimentProfile::Mixed => 0x4528_21e6_38d0_1377,
    }
}

fn put_step(seed: u64, key_id: u64, revision: u64, value_bytes: u32) -> Result<ExperimentStep> {
    let value_len = usize::try_from(value_bytes)
        .map_err(|_| DbError::InvalidInput("experiment value length does not fit usize".to_owned()))?;
    let mut value = vec![0_u8; value_len];
    let mut value_random = SplitMix64::new(
        seed ^ key_id.wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ revision.wrapping_mul(0xbf58_476d_1ce4_e5b9),
    );
    value_random.fill(&mut value);
    Ok(ExperimentStep::Put {
        key: ByteString::from(experiment_key(key_id)),
        value: ByteString::from(value),
    })
}

fn range_step(random: &mut SplitMix64, config: ExperimentConfig) -> ExperimentStep {
    let key_space = u64::from(config.key_space);
    let start_id = random.bounded(key_space);
    let end_id = start_id
        .saturating_add(u64::from(config.range_width))
        .min(key_space);
    ExperimentStep::RangeScan {
        start: ByteString::from(experiment_key(start_id)),
        end: Some(ByteString::from(experiment_key(end_id))),
        limit: config.range_width,
    }
}

fn mixed_step(
    random: &mut SplitMix64,
    config: ExperimentConfig,
    operation_index: u32,
) -> Result<ExperimentStep> {
    let key_id = random.bounded(u64::from(config.key_space));
    match random.bounded(100) {
        0..=29 => put_step(
            config.seed,
            key_id,
            u64::from(operation_index) + 1,
            config.value_bytes,
        ),
        30..=59 => Ok(ExperimentStep::Get {
            key: ByteString::from(experiment_key(key_id)),
        }),
        60..=74 => Ok(ExperimentStep::Delete {
            key: ByteString::from(experiment_key(key_id)),
        }),
        _ => Ok(range_step(random, config)),
    }
}

fn experiment_key(key_id: u64) -> Vec<u8> {
    key_id.to_be_bytes().to_vec()
}

fn validate_experiment_step(step: &ExperimentStep) -> Result<()> {
    match step {
        ExperimentStep::Put { key, value } => validate_key_value(key.as_slice(), value.as_slice()),
        ExperimentStep::Get { key } | ExperimentStep::Delete { key } => validate_key(key.as_slice()),
        ExperimentStep::RangeScan { start, end, limit } => {
            if *limit == 0 || *limit > MAX_EXPERIMENT_RANGE_WIDTH {
                return Err(DbError::InvalidInput(format!(
                    "experiment range limit is {limit}; expected 1..={MAX_EXPERIMENT_RANGE_WIDTH}"
                )));
            }
            validate_range_scan(start.as_slice(), end.as_ref().map(ByteString::as_slice))
        }
        ExperimentStep::Reopen => Ok(()),
    }
}

fn execute_experiment_step<E: KvEngine>(
    engine: &mut E,
    step: &ExperimentStep,
) -> Result<ExperimentOutcome> {
    match step {
        ExperimentStep::Put { key, value } => engine
            .put(key.as_slice(), value.as_slice())
            .map(|previous| ExperimentOutcome::Put {
                previous: previous.map(ByteString::from),
            }),
        ExperimentStep::Get { key } => {
            engine
                .get(key.as_slice())
                .map(|value| ExperimentOutcome::Get {
                    value: value.map(ByteString::from),
                })
        }
        ExperimentStep::Delete { key } => {
            engine
                .delete(key.as_slice())
                .map(|previous| ExperimentOutcome::Delete {
                    previous: previous.map(ByteString::from),
                })
        }
        ExperimentStep::RangeScan { start, end, limit } => {
            let limit = usize::try_from(*limit).map_err(|_| {
                DbError::InvalidInput("experiment range limit does not fit usize".to_owned())
            })?;
            engine
                .range_scan(
                    start.as_slice(),
                    end.as_ref().map(ByteString::as_slice),
                    limit,
                )
                .map(|rows| ExperimentOutcome::RangeScan {
                    rows: rows
                        .into_iter()
                        .map(|(key, value)| (ByteString::from(key), ByteString::from(value)))
                        .collect(),
                })
        }
        ExperimentStep::Reopen => {
            engine.reopen()?;
            Ok(ExperimentOutcome::Reopened)
        }
    }
}

fn execute_matching_phase<L: KvEngine, R: KvEngine>(
    left: &mut L,
    right: &mut R,
    phase: &str,
    steps: &[ExperimentStep],
) -> Result<String> {
    let mut fingerprint = Fnv1a64::new();
    for (index, step) in steps.iter().enumerate() {
        let left_outcome = execute_experiment_step(left, step)?;
        let right_outcome = execute_experiment_step(right, step)?;
        if left_outcome != right_outcome {
            return Err(DbError::InvalidInput(format!(
                "experiment logical mismatch in {phase} step {index} between {} and {}: {left_outcome:?} != {right_outcome:?}",
                left.capabilities().name,
                right.capabilities().name
            )));
        }
        let encoded = serde_json::to_vec(&left_outcome).map_err(|error| {
            DbError::InvalidInput(format!("failed to fingerprint experiment outcome: {error}"))
        })?;
        fingerprint.update_framed(&encoded);
    }
    Ok(fingerprint.finish_hex())
}

fn fingerprint_serializable(value: &impl Serialize) -> Result<String> {
    let encoded = serde_json::to_vec(value).map_err(|error| {
        DbError::InvalidInput(format!("failed to fingerprint experiment input: {error}"))
    })?;
    let mut fingerprint = Fnv1a64::new();
    fingerprint.update(&encoded);
    Ok(fingerprint.finish_hex())
}

struct Fnv1a64 {
    state: u64,
}

impl Fnv1a64 {
    const fn new() -> Self {
        Self {
            state: 0xcbf2_9ce4_8422_2325,
        }
    }

    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.state ^= u64::from(*byte);
            self.state = self.state.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn update_framed(&mut self, bytes: &[u8]) {
        self.update(&u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_le_bytes());
        self.update(bytes);
    }

    fn finish_hex(self) -> String {
        format!("{:016x}", self.state)
    }
}

/// Small specified PRNG chosen so experiment generation is independent of third-party RNG APIs.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn bounded(&mut self, upper_exclusive: u64) -> u64 {
        self.next() % upper_exclusive
    }

    fn fill(&mut self, bytes: &mut [u8]) {
        for chunk in bytes.chunks_mut(8) {
            let random = self.next().to_le_bytes();
            chunk.copy_from_slice(&random[..chunk.len()]);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        generate_experiment_trace, run_amplification_comparison, ExperimentConfig,
        ExperimentProfile, ExperimentStep,
    };
    use crate::{
        validate_key, validate_key_value, validate_range_scan, AmplificationInstrumented,
        AmplificationRatio, AmplificationReport, ConcurrencyMode, CrashRecovery, DistributionMode,
        EngineCapabilities, KvEngine, LogicalModel, Persistence, ReadWorkUnit, Result,
        StorageArchitecture, StructuralReadAmplification, MAX_KEY_BYTES, MAX_VALUE_BYTES,
    };

    const CONFIG: ExperimentConfig = ExperimentConfig {
        seed: 0x5eed_cafe_d15c_a11e,
        operations: 64,
        key_space: 32,
        value_bytes: 24,
        range_width: 5,
        reopen_every: Some(11),
    };

    #[test]
    fn every_profile_is_repeatable_and_records_generator_identity() {
        for profile in [
            ExperimentProfile::PointRead,
            ExperimentProfile::RangeScan,
            ExperimentProfile::SequentialWrite,
            ExperimentProfile::RandomWrite,
            ExperimentProfile::Mixed,
        ] {
            let first = generate_experiment_trace(profile, CONFIG).expect("generate trace");
            let second = generate_experiment_trace(profile, CONFIG).expect("regenerate trace");
            assert_eq!(first, second);
            first.validate().expect("validate generated trace");
            assert_eq!(first.config, CONFIG);
            assert!(!first.measured.is_empty());
        }
    }

    #[test]
    fn read_profiles_seed_state_outside_the_measurement_window() {
        for profile in [
            ExperimentProfile::PointRead,
            ExperimentProfile::RangeScan,
            ExperimentProfile::Mixed,
        ] {
            let trace = generate_experiment_trace(profile, CONFIG).expect("generate trace");
            assert_eq!(trace.setup.len(), CONFIG.key_space as usize + 1);
            assert!(matches!(trace.setup.last(), Some(ExperimentStep::Reopen)));
        }
        for profile in [
            ExperimentProfile::SequentialWrite,
            ExperimentProfile::RandomWrite,
        ] {
            let trace = generate_experiment_trace(profile, CONFIG).expect("generate trace");
            assert!(trace.setup.is_empty());
        }
    }

    #[test]
    fn matching_engines_produce_one_common_outcome_fingerprint() {
        let trace =
            generate_experiment_trace(ExperimentProfile::Mixed, CONFIG).expect("generate trace");
        let mut left = MapEngine::new("left", StorageArchitecture::BPlusTree);
        let mut right = MapEngine::new("right", StorageArchitecture::LsmTree);
        let report = run_amplification_comparison(&mut left, &mut right, &trace)
            .expect("matching engines compare");
        assert_eq!(report.setup_steps_checked, trace.setup.len());
        assert_eq!(report.measured_steps_checked, trace.measured.len());
        assert_eq!(report.left.capabilities.name, "left");
        assert_eq!(report.right.capabilities.name, "right");
        assert_eq!(report.trace_fingerprint.len(), 16);
        assert_eq!(report.measured_outcome_fingerprint.len(), 16);
    }

    #[test]
    fn logical_divergence_fails_closed() {
        let trace = generate_experiment_trace(ExperimentProfile::SequentialWrite, CONFIG)
            .expect("generate trace");
        let mut left = MapEngine::new("left", StorageArchitecture::BPlusTree);
        let mut right = MapEngine::new("right", StorageArchitecture::LsmTree);
        right
            .values
            .insert(0_u64.to_be_bytes().to_vec(), b"different previous value".to_vec());
        let error = run_amplification_comparison(&mut left, &mut right, &trace)
            .expect_err("mismatch must fail");
        assert!(error.to_string().contains("logical mismatch"));
    }

    struct MapEngine {
        name: &'static str,
        architecture: StorageArchitecture,
        values: BTreeMap<Vec<u8>, Vec<u8>>,
        point_reads: u64,
        range_rows: u64,
        logical_write_bytes: u64,
    }

    impl MapEngine {
        fn new(name: &'static str, architecture: StorageArchitecture) -> Self {
            Self {
                name,
                architecture,
                values: BTreeMap::new(),
                point_reads: 0,
                range_rows: 0,
                logical_write_bytes: 0,
            }
        }
    }

    impl KvEngine for MapEngine {
        fn capabilities(&self) -> EngineCapabilities {
            EngineCapabilities {
                name: self.name,
                logical_model: LogicalModel::KeyValue,
                storage_architecture: self.architecture,
                concurrency: ConcurrencyMode::CallerSerialized,
                persistence: Persistence::Persistent,
                crash_recovery: match self.architecture {
                    StorageArchitecture::BPlusTree => CrashRecovery::MirroredCopyOnWritePages,
                    _ => CrashRecovery::WriteAheadLogReplay,
                },
                distribution: DistributionMode::Standalone,
                ordered_range_scan: true,
                max_key_bytes: MAX_KEY_BYTES,
                max_value_bytes: MAX_VALUE_BYTES,
            }
        }

        fn put(&mut self, key: &[u8], value: &[u8]) -> Result<Option<Vec<u8>>> {
            validate_key_value(key, value)?;
            self.logical_write_bytes = self
                .logical_write_bytes
                .saturating_add(u64::try_from(key.len() + value.len()).unwrap_or(u64::MAX));
            Ok(self.values.insert(key.to_vec(), value.to_vec()))
        }

        fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
            validate_key(key)?;
            self.point_reads = self.point_reads.saturating_add(1);
            Ok(self.values.get(key).cloned())
        }

        fn delete(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
            validate_key(key)?;
            self.logical_write_bytes = self
                .logical_write_bytes
                .saturating_add(u64::try_from(key.len()).unwrap_or(u64::MAX));
            Ok(self.values.remove(key))
        }

        fn range_scan(
            &mut self,
            start: &[u8],
            end: Option<&[u8]>,
            limit: usize,
        ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
            validate_range_scan(start, end)?;
            let mut rows = Vec::new();
            for (key, value) in self.values.range(start.to_vec()..) {
                if end.is_some_and(|upper| key.as_slice() >= upper) || rows.len() == limit {
                    break;
                }
                rows.push((key.clone(), value.clone()));
            }
            self.range_rows = self
                .range_rows
                .saturating_add(u64::try_from(rows.len()).unwrap_or(u64::MAX));
            Ok(rows)
        }

        fn reopen(&mut self) -> Result<()> {
            Ok(())
        }
    }

    impl AmplificationInstrumented for MapEngine {
        fn reset_amplification(&mut self) {
            self.point_reads = 0;
            self.range_rows = 0;
            self.logical_write_bytes = 0;
        }

        fn amplification_report(&mut self) -> Result<AmplificationReport> {
            let unit = match self.architecture {
                StorageArchitecture::BPlusTree => ReadWorkUnit::BtreePageAccess,
                _ => ReadWorkUnit::LsmSstableConsult,
            };
            Ok(AmplificationReport {
                point_read: StructuralReadAmplification {
                    ratio: AmplificationRatio {
                        numerator: self.point_reads,
                        denominator: self.point_reads,
                    },
                    unit,
                },
                range_read: StructuralReadAmplification {
                    ratio: AmplificationRatio {
                        numerator: self.range_rows,
                        denominator: self.range_rows,
                    },
                    unit,
                },
                data_write_bytes_per_logical_byte: AmplificationRatio {
                    numerator: self.logical_write_bytes,
                    denominator: self.logical_write_bytes,
                },
                primary_structure_bytes_per_live_byte: AmplificationRatio {
                    numerator: 0,
                    denominator: 0,
                },
            })
        }
    }
}
