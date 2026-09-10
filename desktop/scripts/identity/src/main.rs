// Packaging and the native application use exactly the same schema validator.
#[allow(dead_code)]
#[path = "../../../../crates/claw-core/src/product_identity.rs"]
mod product_identity;

fn main() {
    let identity = product_identity::product_identity();
    println!("{}", serde_json::json!({"display_name": identity.display_name()}));
}
