use crate::{IntentOutputContract, OutputDeliveryIntent, OutputResponseShape};

const GENERATED_FILE_DELIVERY_ATTACHMENT_REPAIR_MARKER: &str =
    "generated_file_delivery_cleared_spurious_attachment_processing";

pub(super) fn clear_spurious_generated_file_delivery_attachment_processing(
    attachment_processing_required: &mut bool,
    output_contract: &IntentOutputContract,
    wants_file_delivery: bool,
) -> Option<&'static str> {
    if !*attachment_processing_required {
        return None;
    }
    let delivery_signal = wants_file_delivery
        || output_contract.delivery_required
        || output_contract.response_shape == OutputResponseShape::FileToken
        || output_contract.delivery_intent == OutputDeliveryIntent::FileSingle;
    if delivery_signal
        && output_contract.delivery_required
        && output_contract.response_shape == OutputResponseShape::FileToken
        && output_contract.delivery_intent == OutputDeliveryIntent::FileSingle
    {
        *attachment_processing_required = false;
        Some(GENERATED_FILE_DELIVERY_ATTACHMENT_REPAIR_MARKER)
    } else {
        None
    }
}
