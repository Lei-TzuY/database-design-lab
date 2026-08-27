//! Shared logical semantics for the database laboratory.
//!
//! This crate deliberately contains only concepts already exercised by real engines. It is not a
//! framework for hypothetical future data models.

mod bytes;
mod differential;
mod engine;
mod error;
mod operation;
mod workload;

pub use bytes::{ByteString, ByteStringError};
pub use differential::{compare_workload, DifferentialError, DifferentialReport};
pub use engine::{
    execute_step, execute_workload, validate_key, validate_key_value, validate_range_scan,
    ConcurrencyMode, CrashRecovery, DistributionMode, EngineCapabilities, KvEngine, LogicalModel,
    Persistence, StorageArchitecture, MAX_KEY_BYTES, MAX_VALUE_BYTES,
};
pub use error::{DbError, ErrorClass, Result};
pub use operation::{Outcome, WorkloadStep};
pub use workload::{
    generate_workload, GeneratorConfig, Workload, MAX_WORKLOAD_STEPS, WORKLOAD_FORMAT_VERSION,
};
