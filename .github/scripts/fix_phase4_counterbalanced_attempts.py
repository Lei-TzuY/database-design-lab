from pathlib import Path

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
