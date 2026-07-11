//! HTTP error translation at the ontology API boundary.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use wyse_ontology::OntologyError;

/// JSON error returned by schema routes.
#[derive(Debug)]
pub(crate) enum ApiError {
    BadRequest(String),
    Conflict,
    Ontology(OntologyError),
}

impl From<OntologyError> for ApiError {
    fn from(value: OntologyError) -> Self {
        Self::Ontology(value)
    }
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostics: Option<&'a [String]>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error, message, diagnostics) = match &self {
            Self::BadRequest(message) => (
                StatusCode::BAD_REQUEST,
                "bad_request",
                Some(message.as_str()),
                None,
            ),
            Self::Conflict => (StatusCode::CONFLICT, "conflict", None, None),
            Self::Ontology(error) => match error {
                OntologyError::DraftMissing { .. }
                | OntologyError::RevisionMissing { .. }
                | OntologyError::TagMissing { .. }
                | OntologyError::ObjectMissing { .. }
                | OntologyError::LinkMissing { .. }
                | OntologyError::ObjectTypeMissing { .. }
                | OntologyError::PropertyTypeMissing { .. }
                | OntologyError::LinkTypeMissing { .. } => {
                    (StatusCode::NOT_FOUND, "not_found", None, None)
                }
                OntologyError::DraftConflict { .. }
                | OntologyError::ObjectVersionConflict { .. }
                | OntologyError::LinkVersionConflict { .. } => (
                    StatusCode::PRECONDITION_FAILED,
                    "precondition_failed",
                    None,
                    None,
                ),
                OntologyError::ObjectReferenced { .. }
                | OntologyError::CardinalityConflict { .. }
                | OntologyError::ReservedTag => (StatusCode::CONFLICT, "conflict", None, None),
                OntologyError::SchemaInvalid { diagnostics } => {
                    let status = if diagnostics
                        .iter()
                        .any(|item| item.starts_with("duplicate "))
                    {
                        StatusCode::CONFLICT
                    } else {
                        StatusCode::UNPROCESSABLE_ENTITY
                    };
                    (status, "schema_invalid", None, Some(diagnostics.as_slice()))
                }
                OntologyError::ValueInvalid { diagnostics }
                | OntologyError::PublishInvalid { diagnostics }
                | OntologyError::LinkEndpointInvalid { diagnostics } => (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "validation_failed",
                    None,
                    Some(diagnostics.as_slice()),
                ),
                OntologyError::InvalidDraftName
                | OntologyError::InvalidTagName
                | OntologyError::InvalidRevisionId
                | OntologyError::RevisionIdentityMismatch { .. }
                | OntologyError::InvalidPageLimit { .. } => (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "invalid_request",
                    None,
                    None,
                ),
                OntologyError::DraftCasUnsupported
                | OntologyError::Repository(_)
                | OntologyError::DecodeSchema(_)
                | OntologyError::EncodeSchema(_)
                | OntologyError::Filesystem(_) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    None,
                    None,
                ),
            },
        };
        (
            status,
            Json(ErrorBody {
                error,
                message,
                diagnostics,
            }),
        )
            .into_response()
    }
}
