from pathlib import Path

# Explicit validation trigger after the workflow learned about this fix helper.
path = Path(__file__).resolve().parents[2] / "crates/db-core/src/experiment.rs"
text = path.read_text()

old = '''    Succeeded {
        setup_steps_executed: usize,
        measured_steps_executed: usize,
        left: ExperimentEngineEvidence,
        right: ExperimentEngineEvidence,
    },
'''
new = '''    Succeeded {
        setup_steps_executed: usize,
        measured_steps_executed: usize,
        left: Box<ExperimentEngineEvidence>,
        right: Box<ExperimentEngineEvidence>,
    },
'''
if text.count(old) != 1:
    raise SystemExit(f"success evidence fields: expected 1 marker, found {text.count(old)}")
text = text.replace(old, new, 1)

old = '''        left_operational_timing: Option<OperationalTimingReport>,
        /// Partial right timing evidence when engines existed before failure.
        right_operational_timing: Option<OperationalTimingReport>,
'''
new = '''        left_operational_timing: Option<Box<OperationalTimingReport>>,
        /// Partial right timing evidence when engines existed before failure.
        right_operational_timing: Option<Box<OperationalTimingReport>>,
'''
if text.count(old) != 1:
    raise SystemExit(f"failure timing fields: expected 1 marker, found {text.count(old)}")
text = text.replace(old, new, 1)

old = '''                            ExperimentAttemptResult::Succeeded {
                                setup_steps_executed: report.setup_steps_executed,
                                measured_steps_executed: report.measured_steps_executed,
                                left: report.left,
                                right: report.right,
                            }
'''
new = '''                            ExperimentAttemptResult::Succeeded {
                                setup_steps_executed: report.setup_steps_executed,
                                measured_steps_executed: report.measured_steps_executed,
                                left: Box::new(report.left),
                                right: Box::new(report.right),
                            }
'''
if text.count(old) != 1:
    raise SystemExit(f"success evidence construction: expected 1 marker, found {text.count(old)}")
text = text.replace(old, new, 1)

text = text.replace(
    "left_operational_timing: Some(left.operational_timing_report()),",
    "left_operational_timing: Some(Box::new(left.operational_timing_report())),",
)
text = text.replace(
    "right_operational_timing: Some(right.operational_timing_report()),",
    "right_operational_timing: Some(Box::new(right.operational_timing_report())),",
)
text = text.replace(
    "left_operational_timing: Some(report.left.operational_timing.clone()),",
    "left_operational_timing: Some(Box::new(report.left.operational_timing.clone())),",
)
text = text.replace(
    "right_operational_timing: Some(report.right.operational_timing.clone()),",
    "right_operational_timing: Some(Box::new(report.right.operational_timing.clone())),",
)

old = '''        if disposition == ExperimentAttemptDisposition::Included {
            match result {
                ExperimentAttemptResult::Succeeded { .. } => {
                    summary.included_successes = summary.included_successes.saturating_add(1)
                }
                ExperimentAttemptResult::Failed { .. } => {
                    summary.included_failures = summary.included_failures.saturating_add(1)
                }
            }
        }
'''
new = '''        if disposition == ExperimentAttemptDisposition::Included {
            match &result {
                ExperimentAttemptResult::Succeeded { .. } => {
                    summary.included_successes = summary.included_successes.saturating_add(1)
                }
                ExperimentAttemptResult::Failed { .. } => {
                    summary.included_failures = summary.included_failures.saturating_add(1)
                }
            }
        }
'''
if text.count(old) != 1:
    raise SystemExit(f"result summary borrow: expected 1 marker, found {text.count(old)}")
text = text.replace(old, new, 1)

text = text.replace(
    "        ExperimentExecutionOrder, ExperimentGeneratorConfig, ExperimentProfile, ExperimentStep,\n",
    "        ExperimentGeneratorConfig, ExperimentProfile, ExperimentStep,\n",
    1,
)

path.write_text(text)
print("applied counterbalanced attempt compile fixes")
