use super::*;

#[test]
fn schema_projection_exposes_bounds_without_argument_values() {
    let schema = json!({
        "type": "object",
        "properties": {
            "latitude": {
                "type": "number",
                "minimum": -90,
                "maximum": 90,
                "description": "provider-specific prose must not enter evidence"
            }
        }
    });

    let field_schema = schema_at_field(&schema, "latitude").expect("latitude schema");
    let projection = safe_schema_projection(field_schema);

    assert_eq!(projection["type"], "number");
    assert_eq!(projection["minimum"], -90);
    assert_eq!(projection["maximum"], 90);
    assert!(projection.get("description").is_none());
}

#[test]
fn nested_array_schema_lookup_is_supported() {
    let schema = json!({
        "type": "object",
        "properties": {
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "count": {"type": "integer", "minimum": 1, "maximum": 20}
                    }
                }
            }
        }
    });

    let field_schema = schema_at_field(&schema, "items[0].count").expect("nested field schema");
    let projection = safe_schema_projection(field_schema);
    assert_eq!(projection["minimum"], 1);
    assert_eq!(projection["maximum"], 20);
}
