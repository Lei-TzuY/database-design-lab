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
    KvEngine, LogicalModel, OperationalTimingInstrumented, OperationalTimingReport,
    OperationalTimingSample, OperationalWork, OperationalWorkUnit, Persistence, ReadWorkUnit,
    StorageArchitecture, StructuralReadAmplification, MAX_KEY_BYTES, MAX_VALUE_BYTES,
};
pub use error::{DbError, ErrorClass, Result};
pub use experiment::{
    compare_experiment_trace, execute_experiment_step, generate_experiment_trace,
    run_experiment_trace, ExperimentComparisonReport, ExperimentEngineEvidence,
    ExperimentGeneratorConfig, ExperimentOutcome, ExperimentProfile, ExperimentRow,
    ExperimentRunReport, ExperimentStep, ExperimentTrace, EXPERIMENT_TRACE_FORMAT_VERSION,
    MAX_EXPERIMENT_OUTCOME_PAYLOAD_BYTES, MAX_EXPERIMENT_RANGE_LIMIT, MAX_EXPERIMENT_STEPS,
    MAX_EXPERIMENT_TRACE_PAYLOAD_BYTES,
};
pub use operation::{Outcome, WorkloadStep};
pub use workload::{
    generate_workload, GeneratorConfig, Workload, MAX_WORKLOAD_STEPS, WORKLOAD_FORMAT_VERSION,
};
