use claw_core::channel_provider_error::{
    ChannelProviderError, ChannelProviderFailureClass, ChannelProviderTransportKind,
};
use teloxide::{ApiError, RequestError};

pub(super) fn telegram_request_error(
    operation: &str,
    error: &RequestError,
) -> ChannelProviderError {
    match error {
        RequestError::RetryAfter(duration) => ChannelProviderError::from_machine_failure(
            "telegram_bot",
            operation,
            ChannelProviderFailureClass::RateLimited,
            Some(429),
            Some("retry_after"),
            Some(duration.as_secs().max(1)),
            "telegram:retry_after",
        ),
        RequestError::Api(api_error) => telegram_api_error(operation, api_error),
        RequestError::Network(_) => ChannelProviderError::from_transport(
            "telegram_bot",
            operation,
            ChannelProviderTransportKind::Request,
            "telegram:network",
        ),
        RequestError::InvalidJson { .. } => ChannelProviderError::invalid_response(
            "telegram_bot",
            operation,
            "telegram:invalid_json",
        ),
        RequestError::Io(_) => ChannelProviderError::from_transport(
            "telegram_bot",
            operation,
            ChannelProviderTransportKind::Body,
            "telegram:io",
        ),
        RequestError::MigrateToChatId(_) => ChannelProviderError::from_machine_failure(
            "telegram_bot",
            operation,
            ChannelProviderFailureClass::TargetNotFound,
            Some(400),
            Some("chat_migrated"),
            None,
            "telegram:chat_migrated",
        ),
    }
}

fn telegram_api_error(operation: &str, error: &ApiError) -> ChannelProviderError {
    let (failure_class, status_code, provider_error_code) = match error {
        ApiError::BotBlocked => (
            ChannelProviderFailureClass::RecipientBlocked,
            403,
            "bot_blocked",
        ),
        ApiError::BotKicked => (
            ChannelProviderFailureClass::RecipientBlocked,
            401,
            "bot_kicked",
        ),
        ApiError::BotKickedFromSupergroup => (
            ChannelProviderFailureClass::RecipientBlocked,
            403,
            "bot_kicked_from_supergroup",
        ),
        ApiError::UserDeactivated => (
            ChannelProviderFailureClass::RecipientBlocked,
            403,
            "user_deactivated",
        ),
        ApiError::CantInitiateConversation => (
            ChannelProviderFailureClass::RecipientBlocked,
            403,
            "cant_initiate_conversation",
        ),
        ApiError::ChatNotFound => (
            ChannelProviderFailureClass::TargetNotFound,
            400,
            "chat_not_found",
        ),
        ApiError::UserNotFound => (
            ChannelProviderFailureClass::TargetNotFound,
            400,
            "user_not_found",
        ),
        ApiError::GroupDeactivated => (
            ChannelProviderFailureClass::TargetNotFound,
            400,
            "group_deactivated",
        ),
        ApiError::NotFound => (
            ChannelProviderFailureClass::Authentication,
            401,
            "invalid_bot_token",
        ),
        ApiError::NotEnoughRightsToPinMessage => (
            ChannelProviderFailureClass::PermissionDenied,
            400,
            "not_enough_rights_to_pin",
        ),
        ApiError::NotEnoughRightsToManagePins => (
            ChannelProviderFailureClass::PermissionDenied,
            400,
            "not_enough_rights_to_manage_pins",
        ),
        ApiError::NotEnoughRightsToChangeChatPermissions => (
            ChannelProviderFailureClass::PermissionDenied,
            400,
            "not_enough_rights_to_change_permissions",
        ),
        ApiError::NotEnoughRightsToRestrict => (
            ChannelProviderFailureClass::PermissionDenied,
            400,
            "not_enough_rights_to_restrict",
        ),
        ApiError::NotEnoughRightsToPostMessages => (
            ChannelProviderFailureClass::PermissionDenied,
            400,
            "not_enough_rights_to_post",
        ),
        ApiError::RequestEntityTooLarge => (
            ChannelProviderFailureClass::PayloadRejected,
            413,
            "request_entity_too_large",
        ),
        _ => (
            ChannelProviderFailureClass::PayloadRejected,
            400,
            "telegram_api_rejected",
        ),
    };
    ChannelProviderError::from_machine_failure(
        "telegram_bot",
        operation,
        failure_class,
        Some(status_code),
        Some(provider_error_code),
        None,
        provider_error_code,
    )
}

#[cfg(test)]
#[path = "telegram_provider_failure_tests.rs"]
mod tests;
