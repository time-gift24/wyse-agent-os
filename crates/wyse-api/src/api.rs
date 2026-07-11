//! HTTP-specific error mapping.

use axum::{
    Json, Router,
    body::Body,
    extract::{
        DefaultBodyLimit, MatchedPath, OriginalUri, Path, Query, State,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::{
        HeaderMap, HeaderValue, Method, Request, StatusCode,
        header::{CONTENT_TYPE, LOCATION},
    },
    response::sse::{Event as SseEvent, KeepAlive},
    response::{IntoResponse, Response, Sse},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    trace::TraceLayer,
};
use tracing::{Span, field, info_span};
use wyse_agent::AgentError;
use wyse_config::{AgentName, ConfigError};
use wyse_core::{
    AgentEvent, AgentId, ApprovalDecision, ApprovalId, ChatMessage, EventCursor, EventRecord,
    HistoryPage, HistoryQuery, ReplayStart, RunId, RuntimeEvent, TokenUsage, TurnId,
};
use wyse_infra::EventStreamBusError;
use wyse_store::{AgentState, AgentStatus, MAX_HISTORY_PAGE_SIZE, StoreError};

use crate::{HostError, HostState};

/// Response returned after an agent and its initial run are durably accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AgentCreated {
    /// New agent identity.
    pub agent_id: AgentId,
    /// Resolved template name.
    pub agent_name: String,
    /// Initial run identity.
    pub run_id: RunId,
}

/// Public projection of one persisted agent state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AgentView {
    /// Agent identity.
    pub agent_id: AgentId,
    /// Resolved template name.
    pub agent_name: String,
    /// Persisted runtime status.
    pub status: AgentStatus,
    /// Current run identity, when present.
    pub run_id: Option<RunId>,
    /// Current turn identity, when present.
    pub turn_id: Option<TurnId>,
    /// Cumulative model token usage.
    pub usage: TokenUsage,
    /// Last committed complete-message sequence.
    pub last_seq: u64,
    /// Last persisted state update.
    pub updated_at: DateTime<Utc>,
}

/// Response returned after a run is durably accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RunAccepted {
    /// Accepted run identity.
    pub run_id: RunId,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateAgentRequest {
    agent_name: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageRequest {
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalRequest {
    decision: ApprovalDecision,
}

#[derive(Deserialize)]
struct HistoryParams {
    #[serde(default)]
    after_seq: u64,
    through_seq: Option<u64>,
    #[serde(default = "default_history_limit")]
    limit: usize,
}

const fn default_history_limit() -> usize {
    100
}

/// Builds the HTTP API router for one host state.
pub fn router(state: Arc<HostState>) -> Router {
    let origins = state
        .allowed_origins()
        .iter()
        .map(|origin| {
            HeaderValue::from_str(origin).expect("allowed origins are validated during parsing")
        })
        .collect::<Vec<_>>();
    let router = Router::new()
        .route("/v1/agents", post(create_agent))
        .route("/v1/agents/{agent_id}", get(get_agent))
        .route(
            "/v1/agents/{agent_id}/messages",
            get(get_messages).post(post_message),
        )
        .route("/v1/agents/{agent_id}/resume", post(resume_agent))
        .route("/v1/agents/{agent_id}/cancel", post(cancel_agent))
        .route(
            "/v1/agents/{agent_id}/approvals/{approval_id}",
            post(resolve_approval),
        )
        .route("/v1/agents/{agent_id}/events", get(get_events))
        .with_state(state)
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<Body>| {
                    let route = request
                        .extensions()
                        .get::<MatchedPath>()
                        .map_or("unmatched", MatchedPath::as_str);
                    info_span!(
                        "http.request",
                        route,
                        method = %request.method(),
                        agent_id = field::Empty,
                        agent_name = field::Empty,
                        run_id = field::Empty,
                        cursor = field::Empty,
                        status = field::Empty,
                        latency = field::Empty,
                    )
                })
                .on_response(
                    |response: &axum::response::Response, latency: Duration, span: &Span| {
                        span.record("status", response.status().as_u16());
                        span.record("latency", field::debug(latency));
                    },
                ),
        );
    if origins.is_empty() {
        router
    } else {
        router.layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::list(origins))
                .allow_methods([Method::GET, Method::POST])
                .allow_headers([CONTENT_TYPE]),
        )
    }
}

fn json_request<T>(request: Result<Json<T>, JsonRejection>) -> Result<T, HostError> {
    request.map(|Json(value)| value).map_err(|rejection| {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            HostError::MessageTooLarge
        } else {
            HostError::InvalidRequest
        }
    })
}

async fn create_agent(
    State(state): State<Arc<HostState>>,
    request: Result<Json<CreateAgentRequest>, JsonRejection>,
) -> Result<Response, HostError> {
    let request = json_request(request)?;
    let agent_name: AgentName = request.agent_name.parse()?;
    Span::current().record("agent_name", agent_name.as_str());
    let created = state.create_agent(agent_name, request.text).await?;
    Span::current().record("agent_id", field::display(created.agent_id));
    Span::current().record("run_id", field::display(created.run_id));
    let location = HeaderValue::from_str(&format!("/v1/agents/{}", created.agent_id))
        .expect("agent id always produces a valid location header");
    let body = AgentCreated {
        agent_id: created.agent_id,
        agent_name: created.agent_name.into(),
        run_id: created.run_id,
    };
    let mut response = (StatusCode::CREATED, Json(body)).into_response();
    response.headers_mut().insert(LOCATION, location);
    Ok(response)
}

async fn get_agent(
    State(state): State<Arc<HostState>>,
    path: Result<Path<AgentId>, PathRejection>,
) -> Result<Json<AgentView>, HostError> {
    let Path(agent_id) = path.map_err(|_| HostError::InvalidRequest)?;
    record_agent_id(agent_id);
    let hosted = find_agent(&state, agent_id)?;
    let persisted = hosted.store.load_agent().await?;
    Ok(Json(persisted.into()))
}

async fn get_messages(
    State(state): State<Arc<HostState>>,
    path: Result<Path<AgentId>, PathRejection>,
    query: Result<Query<HistoryParams>, QueryRejection>,
) -> Result<Json<HistoryPage>, HostError> {
    let Path(agent_id) = path.map_err(|_| HostError::InvalidRequest)?;
    record_agent_id(agent_id);
    let Query(query) = query.map_err(|_| HostError::InvalidHistoryQuery)?;
    let hosted = find_agent(&state, agent_id)?;
    let page = hosted
        .store
        .history_page(HistoryQuery {
            after_seq: query.after_seq,
            through_seq: query.through_seq,
            limit: query.limit,
        })
        .await?;
    Ok(Json(page))
}

async fn get_events(
    State(state): State<Arc<HostState>>,
    path: Result<Path<AgentId>, PathRejection>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<impl IntoResponse, HostError> {
    let Path(agent_id) = path.map_err(|_| HostError::InvalidRequest)?;
    record_agent_id(agent_id);
    find_agent(&state, agent_id)?;
    let replay_start = replay_start(&headers, &uri)?;
    if let ReplayStart::After(cursor) = &replay_start {
        Span::current().record("cursor", cursor.transport_sequence());
    }
    let events = state
        .event_bus()
        .subscribe_agent(agent_id, replay_start)
        .await?;
    let shutdown = state.shutdown_token();
    let events = stream::unfold(Some((events, shutdown)), |state| async move {
        let (mut events, shutdown) = state?;
        tokio::select! {
            biased;
            () = shutdown.cancelled() => None,
            event = events.next() => match event {
                Some(Ok(record)) => match event_record_to_sse(record) {
                    Ok(event) => Some((Ok::<_, Infallible>(event), Some((events, shutdown)))),
                    Err(_) => Some((Ok(stream_error_event()), None)),
                },
                Some(Err(_)) => Some((Ok(stream_error_event()), None)),
                None => None,
            }
        }
    });
    Ok(Sse::new(events).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

fn replay_start(headers: &HeaderMap, uri: &axum::http::Uri) -> Result<ReplayStart, HostError> {
    if let Some(cursor) = headers.get("last-event-id") {
        return parse_cursor(cursor.to_str().map_err(|_| HostError::InvalidCursor)?);
    }
    let Query(params) =
        Query::<Vec<(String, String)>>::try_from_uri(uri).map_err(|_| HostError::InvalidRequest)?;
    let mut after_cursors = params
        .iter()
        .filter(|(key, _)| key == "after_cursor")
        .map(|(_, value)| value.as_str());
    if let Some(cursor) = after_cursors.next() {
        if after_cursors.next().is_some() {
            return Err(HostError::InvalidCursor);
        }
        return parse_cursor(cursor);
    }
    let mut replay = params
        .iter()
        .filter(|(key, _)| key == "replay")
        .map(|(_, value)| value.as_str());
    match (replay.next(), replay.next()) {
        (Some("new"), None) => Ok(ReplayStart::New),
        (Some("all"), None) | (None, None) => Ok(ReplayStart::All),
        _ => Err(HostError::InvalidRequest),
    }
}

fn parse_cursor(cursor: &str) -> Result<ReplayStart, HostError> {
    cursor
        .parse()
        .map(EventCursor::from_transport_sequence)
        .map(ReplayStart::After)
        .map_err(|_| HostError::InvalidCursor)
}

fn event_record_to_sse(record: EventRecord) -> Result<SseEvent, serde_json::Error> {
    let EventRecord { cursor, envelope } = record;
    let event_type = match &envelope.event {
        RuntimeEvent::Agent { event, .. } => event.event_type(),
        event => event.event_type(),
    };
    let data = serde_json::to_string(&envelope)?;
    Ok(SseEvent::default()
        .id(cursor.transport_sequence().to_string())
        .event(event_type)
        .data(data))
}

fn stream_error_event() -> SseEvent {
    SseEvent::default().event("stream_error").data(
        r#"{"error":{"code":"event_stream_unavailable","message":"event stream is unavailable"}}"#,
    )
}

async fn post_message(
    State(state): State<Arc<HostState>>,
    path: Result<Path<AgentId>, PathRejection>,
    request: Result<Json<MessageRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<RunAccepted>), HostError> {
    let Path(agent_id) = path.map_err(|_| HostError::InvalidRequest)?;
    record_agent_id(agent_id);
    let request = json_request(request)?;
    let _admission = state.admit()?;
    if request.text.trim().is_empty() {
        return Err(HostError::InvalidMessage);
    }
    let hosted = find_agent(&state, agent_id)?;
    if hosted.needs_resume() {
        return Err(HostError::ResumeRequired);
    }
    let shutdown = state.shutdown_token();
    let run = hosted.agent.run_turn(ChatMessage::user(request.text));
    let result = tokio::select! {
        biased;
        () = shutdown.cancelled() => {
            hosted.mark_needs_resume();
            hosted.agent.stop();
            return Err(HostError::HostShuttingDown);
        }
        result = run => result,
    };
    let run_id = match result {
        Ok(run_id) => run_id,
        Err(error @ AgentError::RunAlreadyActive) => return Err(error.into()),
        Err(error) => {
            hosted.mark_needs_resume();
            return Err(error.into());
        }
    };
    if state.is_shutting_down() {
        hosted.agent.stop();
        return Err(HostError::HostShuttingDown);
    }
    Span::current().record("run_id", field::display(run_id));
    Ok((StatusCode::ACCEPTED, Json(RunAccepted { run_id })))
}

async fn resume_agent(
    State(state): State<Arc<HostState>>,
    path: Result<Path<AgentId>, PathRejection>,
) -> Result<(StatusCode, Json<RunAccepted>), HostError> {
    let Path(agent_id) = path.map_err(|_| HostError::InvalidRequest)?;
    record_agent_id(agent_id);
    let _admission = state.admit()?;
    let hosted = find_agent(&state, agent_id)?;
    let operation = async {
        let persisted = hosted.store.load_agent().await?;
        if persisted.status != AgentStatus::Running {
            hosted.clear_needs_resume();
            return Err(AgentError::ResumeNotRunning {
                actual: persisted.status,
            }
            .into());
        }
        reconcile_started_only(&hosted, &persisted).await?;
        hosted.agent.resume().await.map_err(HostError::from)
    };
    let shutdown = state.shutdown_token();
    let result = tokio::select! {
        biased;
        () = shutdown.cancelled() => {
            hosted.agent.stop();
            return Err(HostError::HostShuttingDown);
        }
        result = operation => result,
    };
    let run_id = match result {
        Ok(run_id) => run_id,
        Err(error @ HostError::Agent(AgentError::ResumeNotRunning { .. })) => {
            hosted.clear_needs_resume();
            return Err(error);
        }
        Err(error) => return Err(error),
    };
    if state.is_shutting_down() {
        hosted.agent.stop();
        return Err(HostError::HostShuttingDown);
    }
    hosted.clear_needs_resume();
    Span::current().record("run_id", field::display(run_id));
    Ok((StatusCode::ACCEPTED, Json(RunAccepted { run_id })))
}

async fn reconcile_started_only(
    hosted: &crate::HostedAgent,
    persisted: &AgentState,
) -> Result<(), HostError> {
    let (Some(_), Some(current_turn_id)) = (persisted.run_id, persisted.turn_id) else {
        return Ok(());
    };
    let mut after_seq = 0;
    while after_seq < persisted.last_seq {
        let page = hosted
            .store
            .history_page(HistoryQuery {
                after_seq,
                through_seq: Some(persisted.last_seq),
                limit: MAX_HISTORY_PAGE_SIZE,
            })
            .await?;
        if page.through_seq != persisted.last_seq
            || page.events.is_empty()
            || page.next_front_seq <= after_seq
            || page.next_front_seq > persisted.last_seq
        {
            return Err(AgentError::InvalidResumeHistory.into());
        }
        let next_front_seq = page.next_front_seq;
        for envelope in page.events {
            if matches!(
                envelope.event,
                RuntimeEvent::Agent {
                    event: AgentEvent::Message { turn_id, .. },
                    ..
                } if turn_id == current_turn_id
            ) {
                return Ok(());
            }
        }
        after_seq = next_front_seq;
    }

    hosted
        .store
        .update_state(
            AgentStatus::Failed,
            persisted.run_id,
            persisted.turn_id,
            persisted.usage,
        )
        .await?;
    hosted.clear_needs_resume();
    Err(AgentError::ResumeNotRunning {
        actual: AgentStatus::Failed,
    }
    .into())
}

async fn cancel_agent(
    State(state): State<Arc<HostState>>,
    path: Result<Path<AgentId>, PathRejection>,
) -> Result<StatusCode, HostError> {
    let Path(agent_id) = path.map_err(|_| HostError::InvalidRequest)?;
    record_agent_id(agent_id);
    let hosted = find_agent(&state, agent_id)?;
    if hosted.needs_resume() {
        return Err(HostError::ResumeRequired);
    }
    hosted.agent.stop();
    Ok(StatusCode::ACCEPTED)
}

async fn resolve_approval(
    State(state): State<Arc<HostState>>,
    path: Result<Path<(AgentId, ApprovalId)>, PathRejection>,
    request: Result<Json<ApprovalRequest>, JsonRejection>,
) -> Result<StatusCode, HostError> {
    let Path((agent_id, approval_id)) = path.map_err(|_| HostError::InvalidRequest)?;
    record_agent_id(agent_id);
    let request = json_request(request)?;
    let hosted = find_agent(&state, agent_id)?;
    hosted
        .agent
        .resolve_tool_approval(approval_id, request.decision)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn find_agent(state: &HostState, agent_id: AgentId) -> Result<Arc<crate::HostedAgent>, HostError> {
    state
        .agent(agent_id)
        .ok_or(HostError::AgentNotFound { agent_id })
}

fn record_agent_id(agent_id: AgentId) {
    Span::current().record("agent_id", field::display(agent_id));
}

impl From<AgentState> for AgentView {
    fn from(state: AgentState) -> Self {
        Self {
            agent_id: state.agent_id,
            agent_name: state.name,
            status: state.status,
            run_id: state.run_id,
            turn_id: state.turn_id,
            usage: state.usage,
            last_seq: state.last_seq,
            updated_at: state.updated_at,
        }
    }
}

#[derive(Serialize)]
struct ErrorResponse {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: &'static str,
}

impl IntoResponse for HostError {
    fn into_response(self) -> Response {
        let (status, code, message) = error_response(&self);
        if status.is_server_error() {
            tracing::error!(
                http.status = status.as_u16(),
                error.code = code,
                "http request failed"
            );
        }
        (
            status,
            Json(ErrorResponse {
                error: ErrorBody { code, message },
            }),
        )
            .into_response()
    }
}

fn error_response(error: &HostError) -> (StatusCode, &'static str, &'static str) {
    match error {
        HostError::AgentNotFound { .. } => (
            StatusCode::NOT_FOUND,
            "agent_not_found",
            "agent was not found",
        ),
        HostError::TemplateNotFound { .. } => (
            StatusCode::NOT_FOUND,
            "agent_template_not_found",
            "agent template was not found",
        ),
        HostError::InvalidMessage => (
            StatusCode::BAD_REQUEST,
            "invalid_message",
            "message text must not be blank",
        ),
        HostError::InvalidRequest => (
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "request is invalid",
        ),
        HostError::MessageTooLarge => (
            StatusCode::PAYLOAD_TOO_LARGE,
            "message_too_large",
            "request body is too large",
        ),
        HostError::InvalidHistoryQuery => (
            StatusCode::BAD_REQUEST,
            "invalid_history_query",
            "history query is invalid",
        ),
        HostError::InvalidCursor => (
            StatusCode::BAD_REQUEST,
            "invalid_cursor",
            "event cursor is invalid",
        ),
        HostError::ResumeRequired => (
            StatusCode::CONFLICT,
            "resume_required",
            "agent has an unfinished persisted turn",
        ),
        HostError::HostShuttingDown | HostError::CreationStageTimeout => (
            StatusCode::SERVICE_UNAVAILABLE,
            "service_unavailable",
            "service is unavailable",
        ),
        HostError::EmptyText => (
            StatusCode::BAD_REQUEST,
            "invalid_message",
            "initial agent text must not be blank",
        ),
        HostError::ToolNotAvailable { .. } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "tool_not_available",
            "agent template requests an unavailable tool",
        ),
        HostError::Config(ConfigError::InvalidAgentName { .. }) => (
            StatusCode::BAD_REQUEST,
            "invalid_agent_name",
            "agent name is invalid",
        ),
        HostError::Config(ConfigError::ModelNotConfigured { .. }) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "model_not_configured",
            "agent model is not configured",
        ),
        HostError::Config(_) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_agent_template",
            "agent template is invalid",
        ),
        HostError::Agent(AgentError::RunAlreadyActive) => (
            StatusCode::CONFLICT,
            "agent_busy",
            "agent already has an active run",
        ),
        HostError::Agent(AgentError::PersistedRunRequiresResume { .. }) => (
            StatusCode::CONFLICT,
            "resume_required",
            "agent has an unfinished persisted turn",
        ),
        HostError::Agent(AgentError::ApprovalCommandBusy { .. }) => (
            StatusCode::CONFLICT,
            "agent_busy",
            "agent approval command is busy",
        ),
        HostError::Agent(AgentError::ResumeNotRunning { .. }) => (
            StatusCode::CONFLICT,
            "resume_not_running",
            "agent has no persisted running turn",
        ),
        HostError::Agent(AgentError::NoActiveTurn | AgentError::ApprovalNotFound { .. }) => (
            StatusCode::CONFLICT,
            "approval_not_active",
            "tool approval is not active",
        ),
        HostError::Agent(AgentError::MissingBuilderField { .. })
        | HostError::Llm(_)
        | HostError::Tool(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "agent_initialization_failed",
            "agent initialization failed",
        ),
        HostError::Store(source) | HostError::Agent(AgentError::Store { source }) => {
            store_error_response(source)
        }
        HostError::Agent(AgentError::EventBus {
            source: EventStreamBusError::Persistence { source },
        }) => source
            .downcast_ref::<StoreError>()
            .map_or_else(internal_error_response, store_error_response),
        HostError::EventStreamBus(EventStreamBusError::CursorExpired { .. }) => (
            StatusCode::GONE,
            "cursor_expired",
            "event cursor is no longer retained",
        ),
        HostError::EventStreamBus(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "event_stream_unavailable",
            "event stream is unavailable",
        ),
        HostError::Filesystem(source) => filesystem_store_error_response(source),
        _ => internal_error_response(),
    }
}

fn store_error_response(error: &StoreError) -> (StatusCode, &'static str, &'static str) {
    match error {
        StoreError::InvalidHistoryLimit { .. }
        | StoreError::InvalidHistoryRange { .. }
        | StoreError::HistoryBarrierBeyondLast { .. } => (
            StatusCode::BAD_REQUEST,
            "invalid_history_query",
            "history query is invalid",
        ),
        StoreError::Filesystem(source) => filesystem_store_error_response(source),
        StoreError::CasTimeout | StoreError::CasRetriesExhausted => (
            StatusCode::SERVICE_UNAVAILABLE,
            "store_unavailable",
            "agent store is unavailable",
        ),
        _ => internal_error_response(),
    }
}

fn filesystem_store_error_response(
    error: &wyse_filesystem::FilesystemError,
) -> (StatusCode, &'static str, &'static str) {
    match error {
        wyse_filesystem::FilesystemError::PermissionDenied { .. }
        | wyse_filesystem::FilesystemError::LocalIo { .. } => (
            StatusCode::SERVICE_UNAVAILABLE,
            "store_unavailable",
            "agent store is unavailable",
        ),
        _ => internal_error_response(),
    }
}

fn internal_error_response() -> (StatusCode, &'static str, &'static str) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "internal server error",
    )
}
