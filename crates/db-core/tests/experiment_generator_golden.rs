use db_core::{generate_experiment_trace, ExperimentConfig, ExperimentProfile};

const GOLDEN_CONFIG: ExperimentConfig = ExperimentConfig {
    seed: 15_111_065_706_836_454_659,
    operations: 256,
    key_space: 512,
    value_bytes: 512,
    range_width: 32,
    reopen_every: Some(64),
};

#[test]
fn generator_revision_one_has_stable_trace_fingerprints() {
    let cases = [
        (ExperimentProfile::PointRead, "e0a32423aac3071e"),
        (ExperimentProfile::RangeScan, "60855cdb1b32d0b4"),
        (ExperimentProfile::SequentialWrite, "61fd34f1454685d9"),
        (ExperimentProfile::RandomWrite, "d808bd8147e1fbdc"),
        (ExperimentProfile::Mixed, "52e9bb5ec7742d95"),
    ];

    for (profile, expected) in cases {
        let trace =
            generate_experiment_trace(profile, GOLDEN_CONFIG).expect("generate golden trace");
        let encoded = serde_json::to_vec(&trace).expect("serialize golden trace");
        assert_eq!(fnv1a64_hex(&encoded), expected, "profile {profile:?}");
    }
}

fn fnv1a64_hex(bytes: &[u8]) -> String {
    let mut state = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        state ^= u64::from(*byte);
        state = state.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{state:016x}")
}
