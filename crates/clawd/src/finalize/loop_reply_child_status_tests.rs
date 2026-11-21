use super::*;
use serde_json::json;

#[test]
fn mixed_child_statuses_and_evidence_survive_overall_failure_projection() {
    let payload = json!({"status":"blocked","child_task_merge":{"child_task_graph":{"nodes":[
        {"child_task_id":"parent:child:a","role":"explorer","task_status":"succeeded","execution_state":"succeeded",
         "runtime":{"child_task_result":{"error_code":null,"evidence_refs":["task:a:result"],"finding_refs":["child_task:a:structured_result"],"structured_result":{"name":"observed_component","line":9}}}},
        {"child_task_id":"parent:child:b","role":"explorer","task_status":"failed","execution_state":"failed",
         "runtime":{"child_task_result":{"error_code":"evidence_rejected","evidence_refs":["task:b:verification"]}}}
    ]}}});
    let statuses = child_task_status_projection(&payload).unwrap();
    assert_eq!(statuses[0]["status"], "succeeded");
    assert_eq!(statuses[1]["status"], "failed");
    assert_eq!(statuses[0]["evidence_refs"][0], "task:a:result");
    assert_eq!(statuses[1]["error_code"], "evidence_rejected");
    assert_eq!(
        statuses[0]["structured_result"]["name"],
        "observed_component"
    );
    assert_eq!(statuses[0]["structured_result_projection_limited"], false);
    assert!(child_task_status_projection(&json!({"status":"failed"})).is_none());
}

#[test]
fn failure_projection_keeps_refs_when_structured_findings_exceed_inline_bound() {
    let payload = json!({"child_task_merge":{"child_task_graph":{"nodes":[{
        "child_task_id":"parent:child:large","task_status":"succeeded","runtime":{
            "child_task_result":{"structured_result":{"content":"x".repeat(4097)},
                "finding_refs":["child_task:large:structured_result"]}}
    }]}}});
    let result = child_task_status_projection(&payload).unwrap();
    assert!(result[0]["structured_result"].is_null());
    assert_eq!(result[0]["structured_result_projection_limited"], true);
    assert_eq!(
        result[0]["finding_refs"][0],
        "child_task:large:structured_result"
    );
}
