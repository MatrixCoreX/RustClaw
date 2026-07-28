use axum::{http::StatusCode, Json};
use claw_core::types::ApiResponse;

use crate::ui_attachments::UiAttachmentConstraints;

pub(crate) async fn get_ui_attachment_constraints(
) -> (StatusCode, Json<ApiResponse<UiAttachmentConstraints>>) {
    crate::api_ok(crate::ui_attachments::ui_attachment_constraints())
}
