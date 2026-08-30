use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Parser;
use serde::Serialize;
use thiserror::Error;

const PREFLIGHT_PROTOCOL: &str = "linux_controlled_host_preflight_v1";
const EXPECTED_GOVERNOR: &str = "performance";
const MAX_TEXT_BYTES: u64 = 64 * 1024;
const MAX_ATTESTATION_BYTES: usize = 4096;
const MAX_CPU_ID: u32 = 1_048_575;

#[derive(Debug, Parser)]
#[command(
    name = "db-lab-host-preflight",
    version,
    about = "Record a fail-closed Linux controlled-host preflight snapshot"
)]
struct Cli {
    /// Fresh JSON output path. Failed preflights are also retained with passed=false.
    #[arg(long)]
    output: PathBuf,
    /// Stable human-readable identity for the dedicated performance host.
    #[arg(long)]
    host_label: String,
    /// Exact process CPU affinity required for the run, for example 2-5 or 2,4,6.
    #[arg(long)]
    expected_cpus: String,
    /// Maximum acceptable one-minute system load divided by the pinned CPU count.
    #[arg(long)]
    max_load_per_cpu: f64,
    /// Operator statement describing how CPU temperature/thermal equilibrium is controlled.
    #[arg(long)]
    thermal_control_attestation: String,
    /// Operator statement describing how unrelated services/processes are controlled.
    #[arg(long)]
    background_load_attestation: String,
    /// Operator statement describing filesystem/controller/device cache handling.
    #[arg(long)]
    storage_cache_attestation: String,
}

#[derive(Debug, Clone)]
struct PreflightConfig {
    host_label: String,
    expected_cpus: BTreeSet<u32>,
    max_load_per_cpu: f64,
    thermal_control_attestation: String,
    background_load_attestation: String,
    storage_cache_attestation: String,
}

#[derive(Debug, Clone, Serialize)]
struct HostObservation {
    target_os: &'static str,
    target_arch: &'static str,
    kernel_release: Option<String>,
    cpu_model: Option<String>,
    process_allowed_cpus: Option<Vec<u32>>,
    online_cpus: Option<Vec<u32>>,
    governors: BTreeMap<u32, Option<String>>,
    turbo: TurboObservation,
    load_one: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct TurboObservation {
    interface: Option<String>,
    raw_value: Option<String>,
    disabled: Option<bool>,
}

#[derive(Debug, Serialize)]
struct PreflightSnapshot {
    protocol: &'static str,
    recorded_unix_seconds: u64,
    host_label: String,
    passed: bool,
    expected: ExpectedControls,
    observation: HostObservation,
    operator_attestations: OperatorAttestations,
    violations: Vec<String>,
    limitations: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
struct ExpectedControls {
    process_cpu_affinity: Vec<u32>,
    scaling_governor: &'static str,
    turbo_disabled: bool,
    max_load_per_cpu: f64,
}

#[derive(Debug, Serialize)]
struct OperatorAttestations {
    thermal_control: String,
    background_load_control: String,
    storage_cache_control: String,
}

#[derive(Debug, Error)]
enum PreflightError {
    #[error("invalid preflight configuration: {0}")]
    InvalidConfig(String),
    #[error("host preflight is supported only on Linux")]
    UnsupportedPlatform,
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to encode preflight JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("preflight failed with {0} control violation(s)")]
    Failed(usize),
}

fn main() -> ExitCode {
    let result = run(Cli::parse());
    match result {
        Ok(snapshot) => {
            println!(
                "{{\"passed\":true,\"protocol\":\"{}\",\"output\":\"{}\"}}",
                PREFLIGHT_PROTOCOL,
                snapshot.display()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}

fn run(args: Cli) -> Result<PathBuf, PreflightError> {
    let config = parse_config(&args)?;
    ensure_fresh_output(&args.output)?;
    let observation = collect_host_observation(&config.expected_cpus)?;
    let violations = evaluate_controls(&config, &observation);
    let snapshot = PreflightSnapshot {
        protocol: PREFLIGHT_PROTOCOL,
        recorded_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| {
                PreflightError::InvalidConfig(format!("system clock precedes Unix epoch: {error}"))
            })?
            .as_secs(),
        host_label: config.host_label.clone(),
        passed: violations.is_empty(),
        expected: ExpectedControls {
            process_cpu_affinity: config.expected_cpus.iter().copied().collect(),
            scaling_governor: EXPECTED_GOVERNOR,
            turbo_disabled: true,
            max_load_per_cpu: config.max_load_per_cpu,
        },
        observation,
        operator_attestations: OperatorAttestations {
            thermal_control: config.thermal_control_attestation.clone(),
            background_load_control: config.background_load_attestation.clone(),
            storage_cache_control: config.storage_cache_attestation.clone(),
        },
        violations,
        limitations: vec![
            "operator attestations are recorded statements, not independently verified facts",
            "thermal equilibrium and storage/controller cache state are not portable kernel observations in this protocol",
            "a passing snapshot is a prerequisite for controlled collection, not a performance result or regression threshold",
        ],
    };
    write_new_json(&args.output, &snapshot)?;
    if snapshot.passed {
        Ok(args.output)
    } else {
        for violation in &snapshot.violations {
            eprintln!("violation: {violation}");
        }
        Err(PreflightError::Failed(snapshot.violations.len()))
    }
}

fn parse_config(args: &Cli) -> Result<PreflightConfig, PreflightError> {
    let host_label = bounded_text("--host-label", &args.host_label)?;
    let expected_cpus = parse_cpu_list(&args.expected_cpus)?;
    if expected_cpus.is_empty() {
        return Err(PreflightError::InvalidConfig(
            "--expected-cpus must select at least one CPU".to_owned(),
        ));
    }
    if !args.max_load_per_cpu.is_finite() || args.max_load_per_cpu < 0.0 {
        return Err(PreflightError::InvalidConfig(
            "--max-load-per-cpu must be a finite number greater than or equal to zero".to_owned(),
        ));
    }
    Ok(PreflightConfig {
        host_label,
        expected_cpus,
        max_load_per_cpu: args.max_load_per_cpu,
        thermal_control_attestation: bounded_text(
            "--thermal-control-attestation",
            &args.thermal_control_attestation,
        )?,
        background_load_attestation: bounded_text(
            "--background-load-attestation",
            &args.background_load_attestation,
        )?,
        storage_cache_attestation: bounded_text(
            "--storage-cache-attestation",
            &args.storage_cache_attestation,
        )?,
    })
}

fn bounded_text(label: &str, value: &str) -> Result<String, PreflightError> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_ATTESTATION_BYTES {
        return Err(PreflightError::InvalidConfig(format!(
            "{label} must contain 1..={MAX_ATTESTATION_BYTES} UTF-8 bytes after trimming"
        )));
    }
    Ok(value.to_owned())
}

fn parse_cpu_list(value: &str) -> Result<BTreeSet<u32>, PreflightError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(PreflightError::InvalidConfig(
            "CPU list must not be empty".to_owned(),
        ));
    }
    let mut cpus = BTreeSet::new();
    for component in value.split(',') {
        let component = component.trim();
        if component.is_empty() {
            return Err(PreflightError::InvalidConfig(format!(
                "CPU list {value:?} contains an empty component"
            )));
        }
        if let Some((start, end)) = component.split_once('-') {
            if end.contains('-') {
                return Err(PreflightError::InvalidConfig(format!(
                    "CPU range {component:?} is malformed"
                )));
            }
            let start = parse_cpu_id(start, component)?;
            let end = parse_cpu_id(end, component)?;
            if start > end {
                return Err(PreflightError::InvalidConfig(format!(
                    "CPU range {component:?} is descending"
                )));
            }
            for cpu in start..=end {
                if !cpus.insert(cpu) {
                    return Err(PreflightError::InvalidConfig(format!(
                        "CPU {cpu} is selected more than once"
                    )));
                }
            }
        } else {
            let cpu = parse_cpu_id(component, component)?;
            if !cpus.insert(cpu) {
                return Err(PreflightError::InvalidConfig(format!(
                    "CPU {cpu} is selected more than once"
                )));
            }
        }
    }
    Ok(cpus)
}

fn parse_cpu_id(value: &str, component: &str) -> Result<u32, PreflightError> {
    let cpu = value.trim().parse::<u32>().map_err(|_| {
        PreflightError::InvalidConfig(format!("CPU component {component:?} is not numeric"))
    })?;
    if cpu > MAX_CPU_ID {
        return Err(PreflightError::InvalidConfig(format!(
            "CPU id {cpu} exceeds supported maximum {MAX_CPU_ID}"
        )));
    }
    Ok(cpu)
}

fn evaluate_controls(config: &PreflightConfig, observation: &HostObservation) -> Vec<String> {
    let mut violations = Vec::new();
    let expected: Vec<u32> = config.expected_cpus.iter().copied().collect();

    match observation.process_allowed_cpus.as_ref() {
        Some(actual) if actual == &expected => {}
        Some(actual) => violations.push(format!(
            "process CPU affinity is {actual:?}; expected exactly {expected:?}"
        )),
        None => violations.push("process CPU affinity could not be observed".to_owned()),
    }

    match observation.online_cpus.as_ref() {
        Some(actual) => {
            let actual: BTreeSet<u32> = actual.iter().copied().collect();
            let missing: Vec<u32> = config.expected_cpus.difference(&actual).copied().collect();
            if !missing.is_empty() {
                violations.push(format!("expected CPUs are offline or absent: {missing:?}"));
            }
        }
        None => violations.push("online CPU set could not be observed".to_owned()),
    }

    for cpu in &config.expected_cpus {
        match observation.governors.get(cpu).and_then(Option::as_deref) {
            Some(EXPECTED_GOVERNOR) => {}
            Some(actual) => violations.push(format!(
                "CPU {cpu} scaling governor is {actual:?}; expected {EXPECTED_GOVERNOR:?}"
            )),
            None => violations.push(format!("CPU {cpu} scaling governor could not be observed")),
        }
    }

    match observation.turbo.disabled {
        Some(true) => {}
        Some(false) => violations.push(format!(
            "turbo/boost is enabled according to {:?} value {:?}",
            observation.turbo.interface, observation.turbo.raw_value
        )),
        None => violations.push(
            "turbo/boost disabled state could not be established from supported Linux interfaces"
                .to_owned(),
        ),
    }

    match observation.load_one {
        Some(load_one) if load_one.is_finite() => {
            let load_per_cpu = load_one / config.expected_cpus.len() as f64;
            if load_per_cpu > config.max_load_per_cpu {
                violations.push(format!(
                    "one-minute load per pinned CPU is {load_per_cpu:.6}; budget is {:.6}",
                    config.max_load_per_cpu
                ));
            }
        }
        _ => violations.push("one-minute system load could not be observed".to_owned()),
    }

    violations
}

fn ensure_fresh_output(path: &Path) -> Result<(), PreflightError> {
    if path.exists() {
        return Err(PreflightError::InvalidConfig(format!(
            "preflight output already exists: {}",
            path.display()
        )));
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let metadata = fs::symlink_metadata(parent).map_err(|source| PreflightError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    if !metadata.file_type().is_dir() {
        return Err(PreflightError::InvalidConfig(format!(
            "preflight output parent must be a real directory: {}",
            parent.display()
        )));
    }
    Ok(())
}

fn write_new_json(path: &Path, value: &impl Serialize) -> Result<(), PreflightError> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| PreflightError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n").map_err(|source| PreflightError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    writer.flush().map_err(|source| PreflightError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|source| PreflightError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn collect_host_observation(expected_cpus: &BTreeSet<u32>) -> Result<HostObservation, PreflightError> {
    let process_allowed_cpus = read_optional_text(Path::new("/proc/self/status"))?
        .as_deref()
        .and_then(parse_allowed_cpu_line);
    let online_cpus = read_optional_text(Path::new("/sys/devices/system/cpu/online"))?
        .and_then(|value| parse_cpu_list(&value).ok())
        .map(|cpus| cpus.into_iter().collect());

    let mut governors = BTreeMap::new();
    for cpu in expected_cpus {
        let path = PathBuf::from(format!(
            "/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor"
        ));
        governors.insert(*cpu, read_optional_text(&path)?.map(|value| value.trim().to_owned()));
    }

    let turbo = observe_turbo()?;
    let load_one = read_optional_text(Path::new("/proc/loadavg"))?
        .as_deref()
        .and_then(parse_load_one);
    let kernel_release = read_optional_text(Path::new("/proc/sys/kernel/osrelease"))?
        .map(|value| value.trim().to_owned());
    let cpu_model = read_optional_text(Path::new("/proc/cpuinfo"))?
        .as_deref()
        .and_then(parse_cpu_model);

    Ok(HostObservation {
        target_os: std::env::consts::OS,
        target_arch: std::env::consts::ARCH,
        kernel_release,
        cpu_model,
        process_allowed_cpus,
        online_cpus,
        governors,
        turbo,
        load_one,
    })
}

#[cfg(not(target_os = "linux"))]
fn collect_host_observation(
    _expected_cpus: &BTreeSet<u32>,
) -> Result<HostObservation, PreflightError> {
    Err(PreflightError::UnsupportedPlatform)
}

#[cfg(target_os = "linux")]
fn observe_turbo() -> Result<TurboObservation, PreflightError> {
    let candidates = [
        ("/sys/devices/system/cpu/intel_pstate/no_turbo", "1"),
        ("/sys/devices/system/cpu/cpufreq/boost", "0"),
    ];
    for (path, disabled_value) in candidates {
        let path = Path::new(path);
        if let Some(raw) = read_optional_text(path)? {
            let raw = raw.trim().to_owned();
            return Ok(TurboObservation {
                interface: Some(path.display().to_string()),
                disabled: Some(raw == disabled_value),
                raw_value: Some(raw),
            });
        }
    }
    Ok(TurboObservation {
        interface: None,
        raw_value: None,
        disabled: None,
    })
}

#[cfg(target_os = "linux")]
fn read_optional_text(path: &Path) -> Result<Option<String>, PreflightError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(PreflightError::Io {
                path: path.to_path_buf(),
                source,
            })
        }
    };
    if !metadata.file_type().is_file() {
        return Ok(None);
    }
    if metadata.len() > MAX_TEXT_BYTES {
        return Err(PreflightError::InvalidConfig(format!(
            "observable host file {} has {} bytes; maximum is {MAX_TEXT_BYTES}",
            path.display(),
            metadata.len()
        )));
    }
    fs::read_to_string(path)
        .map(Some)
        .map_err(|source| PreflightError::Io {
            path: path.to_path_buf(),
            source,
        })
}

fn parse_allowed_cpu_line(status: &str) -> Option<Vec<u32>> {
    let encoded = status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))?
        .trim();
    parse_cpu_list(encoded)
        .ok()
        .map(|cpus| cpus.into_iter().collect())
}

fn parse_load_one(loadavg: &str) -> Option<f64> {
    loadavg.split_whitespace().next()?.parse::<f64>().ok()
}

fn parse_cpu_model(cpuinfo: &str) -> Option<String> {
    for key in ["model name", "Hardware", "Processor"] {
        if let Some(value) = cpuinfo.lines().find_map(|line| {
            let (found_key, value) = line.split_once(':')?;
            (found_key.trim() == key).then(|| value.trim())
        }) {
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::{
        evaluate_controls, parse_allowed_cpu_line, parse_cpu_list, parse_cpu_model, parse_load_one,
        HostObservation, PreflightConfig, TurboObservation,
    };

    #[test]
    fn cpu_list_parser_is_exact_and_rejects_overlap() {
        assert_eq!(
            parse_cpu_list("0-2,4,6-7").expect("parse cpu list"),
            BTreeSet::from([0, 1, 2, 4, 6, 7])
        );
        assert!(parse_cpu_list("2-1").is_err());
        assert!(parse_cpu_list("1,1").is_err());
        assert!(parse_cpu_list("1-2,2").is_err());
        assert!(parse_cpu_list("1,,2").is_err());
    }

    #[test]
    fn proc_parsers_are_deterministic() {
        let status = "Name:\tdb-lab\nCpus_allowed_list:\t2-3,6\n";
        assert_eq!(parse_allowed_cpu_line(status), Some(vec![2, 3, 6]));
        assert_eq!(parse_load_one("0.42 0.30 0.20 1/100 123"), Some(0.42));
        assert_eq!(
            parse_cpu_model("processor: 0\nmodel name : Example CPU 9000\n"),
            Some("Example CPU 9000".to_owned())
        );
    }

    #[test]
    fn controlled_observation_passes_only_exact_hard_controls() {
        let config = config();
        let observation = valid_observation();
        assert!(evaluate_controls(&config, &observation).is_empty());
    }

    #[test]
    fn affinity_governor_turbo_online_and_load_fail_closed() {
        let config = config();
        let mut observation = valid_observation();
        observation.process_allowed_cpus = Some(vec![2, 3, 4]);
        observation.online_cpus = Some(vec![0, 1, 2]);
        observation.governors.insert(2, Some("powersave".to_owned()));
        observation.governors.insert(3, None);
        observation.turbo.disabled = Some(false);
        observation.load_one = Some(1.0);
        let violations = evaluate_controls(&config, &observation);
        assert_eq!(violations.len(), 6);
        assert!(violations.iter().any(|value| value.contains("affinity")));
        assert!(violations.iter().any(|value| value.contains("offline")));
        assert!(violations.iter().any(|value| value.contains("governor")));
        assert!(violations.iter().any(|value| value.contains("turbo/boost")));
        assert!(violations.iter().any(|value| value.contains("load per pinned CPU")));
    }

    #[test]
    fn missing_observations_are_violations_not_assumptions() {
        let config = config();
        let observation = HostObservation {
            target_os: "linux",
            target_arch: "x86_64",
            kernel_release: None,
            cpu_model: None,
            process_allowed_cpus: None,
            online_cpus: None,
            governors: BTreeMap::new(),
            turbo: TurboObservation {
                interface: None,
                raw_value: None,
                disabled: None,
            },
            load_one: None,
        };
        let violations = evaluate_controls(&config, &observation);
        assert_eq!(violations.len(), 6);
    }

    fn config() -> PreflightConfig {
        PreflightConfig {
            host_label: "perf-host-01".to_owned(),
            expected_cpus: BTreeSet::from([2, 3]),
            max_load_per_cpu: 0.10,
            thermal_control_attestation: "steady-state thermal protocol".to_owned(),
            background_load_attestation: "benchmark services only".to_owned(),
            storage_cache_attestation: "trace-induced warm policy".to_owned(),
        }
    }

    fn valid_observation() -> HostObservation {
        HostObservation {
            target_os: "linux",
            target_arch: "x86_64",
            kernel_release: Some("example".to_owned()),
            cpu_model: Some("Example CPU".to_owned()),
            process_allowed_cpus: Some(vec![2, 3]),
            online_cpus: Some(vec![0, 1, 2, 3]),
            governors: BTreeMap::from([
                (2, Some("performance".to_owned())),
                (3, Some("performance".to_owned())),
            ]),
            turbo: TurboObservation {
                interface: Some("/sys/example".to_owned()),
                raw_value: Some("1".to_owned()),
                disabled: Some(true),
            },
            load_one: Some(0.10),
        }
    }
}
