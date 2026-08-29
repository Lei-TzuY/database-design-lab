//! Shared logical semantics for the database laboratory.
//!
//! This crate deliberately contains only concepts already exercised by real engines. It is not a
//! framework for hypothetical future data models.

mod bytes;
mod differential;
mod engine;
mod error;
mod experiment;
mod operation;
mod workload;

pub use bytes::{ByteString, ByteStringError};
pub use differential::{compare_workload, DifferentialError, DifferentialReport};
pub use engine::{
    execute_step, execute_workload, validate_experiment_compatibility, validate_key,
    validate_key_value, validate_range_scan, AmplificationInstrumented, AmplificationRatio,
    AmplificationReport, ConcurrencyMode, CrashRecovery, DistributionMode, EngineCapabilities,
    KvEngine, LogicalModel, Persistence, ReadWorkUnit, StorageArchitecture,
    StructuralReadAmplification, MAX_KEY_BYTES, MAX_VALUE_BYTES,
};
pub use error::{DbError, ErrorClass, Result};
pub use experiment::{
    generate_experiment_trace, run_amplification_comparison, EngineAmplificationEvidence,
    ExperimentComparisonReport, ExperimentConfig, ExperimentOutcome, ExperimentProfile,
    ExperimentStep, ExperimentTrace, EXPERIMENT_GENERATOR_REVISION, EXPERIMENT_TRACE_FORMAT_VERSION,
    MAX_EXPERIMENT_KEY_SPACE, MAX_EXPERIMENT_RANGE_WIDTH,
};
pub use operation::{Outcome, WorkloadStep};
pub use workload::{
    generate_workload, GeneratorConfig, Workload, MAX_WORKLOAD_STEPS, WORKLOAD_FORMAT_VERSION,
};
