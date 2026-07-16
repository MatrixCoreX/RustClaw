use super::{drop_execution_summaries_when_delivery_is_scalar, route_result};

#[test]
fn config_validation_contract_drops_execution_summary_messages() {
    let mut route = route_result();
    route.semantic_kind = crate::OutputSemanticKind::ConfigValidation;
    route.response_shape = crate::OutputResponseShape::Free;
    let mut messages = vec![
        "**执行过程**\n1. 调用技能 `config_basic`\n   输出：ok".to_string(),
        "validation_status=pass".to_string(),
    ];

    drop_execution_summaries_when_delivery_is_scalar(
        &route,
        "validation_status=pass",
        &mut messages,
    );

    assert_eq!(messages, vec!["validation_status=pass".to_string()]);
}
