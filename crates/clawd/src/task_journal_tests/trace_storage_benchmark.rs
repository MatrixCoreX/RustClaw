use super::*;
use std::hint::black_box;
use std::time::Instant;

fn measure(operation: fn(Value) -> Value, fixture: &Value) -> u128 {
    let input = fixture.clone();
    let started = Instant::now();
    black_box(operation(black_box(input)));
    started.elapsed().as_nanos()
}

#[test]
#[ignore = "manual serial trace-storage benchmark; run with --ignored --nocapture --test-threads=1"]
fn compare_trace_storage_with_frozen_reference() {
    let fixtures = [
        (
            "small",
            json!({"step_results":[{"data":"ready"}],"capability_results":[artifact_fixture(0,0)]}),
        ),
        (
            "near_bound",
            json!({"step_results":[{"output":"x".repeat(40_000)}],"capability_results":[{"data":{"text":"y".repeat(40_000)}}]}),
        ),
        (
            "large_text",
            json!({"step_results":[{"output":"x".repeat(1_000_000)}],"capability_results":[{"data":{"text":"y".repeat(1_000_000)}}]}),
        ),
        (
            "many_artifacts",
            json!({"step_results":[],"capability_results":(0..4000).map(|n| artifact_fixture(n%5,n)).collect::<Vec<_>>()}),
        ),
        (
            "no_artifacts",
            json!({"step_results":[],"capability_results":(0..4000).map(|n| artifact_fixture(0,n)).collect::<Vec<_>>()}),
        ),
    ];
    for (name, fixture) in fixtures {
        assert_projection_equivalent(fixture.clone());
        let mut baseline = Vec::new();
        let mut current = Vec::new();
        for round in 0..11 {
            let (old, new) = if round % 2 == 0 {
                (
                    measure(reference::result_trace_json_with_storage_limit, &fixture),
                    measure(result_trace_json_with_storage_limit, &fixture),
                )
            } else {
                let new = measure(result_trace_json_with_storage_limit, &fixture);
                (
                    measure(reference::result_trace_json_with_storage_limit, &fixture),
                    new,
                )
            };
            if round > 0 {
                baseline.push(old);
                current.push(new);
            }
        }
        baseline.sort_unstable();
        current.sort_unstable();
        let (old, new) = (baseline[baseline.len() / 2], current[current.len() / 2]);
        eprintln!(
            "{}",
            json!({"benchmark":"trace_storage", "fixture":name,"input_bytes":serde_json::to_vec(&fixture).unwrap().len(),"debug_assertions":cfg!(debug_assertions),"samples":baseline.len(),"reference_median_ns":old,"current_median_ns":new,"current_over_reference":new as f64 / old.max(1) as f64,"equivalent":true})
        );
    }
}
