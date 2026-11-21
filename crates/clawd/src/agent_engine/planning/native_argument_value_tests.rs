use super::*;
use serde_json::json;

#[test]
fn typed_native_arguments_reject_invalid_enum_type_and_bounds_before_dispatch() {
    let schema = json!({"type":"object","properties":{
        "mode":{"type":"string","enum":["head","tail","range"]},
        "start_line":{"type":"integer","minimum":1}
    }});
    assert!(native_argument_values_valid(
        &schema,
        &json!({"mode":"range","start_line":1})
    ));
    for args in [
        json!({"mode":"full"}),
        json!({"start_line":"one"}),
        json!({"start_line":0}),
    ] {
        assert!(!native_argument_values_valid(&schema, &args));
    }
}
