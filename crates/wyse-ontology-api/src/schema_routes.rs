//! Draft, revision, tag, and schema graph HTTP routes.

use std::str::FromStr;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use serde::{Deserialize, Serialize};
use wyse_ontology::{
    Cardinality, Draft, DraftName, GraphProjection, ObjectTypeId, PublishedRevision, RevisionId,
    SchemaDocument, SchemaRef, TagName, ValueType,
};

use crate::{AppState, error::ApiError};

/// Creates the `/v1/ontology` schema routes.
pub fn schema_routes() -> Router<AppState> {
    Router::new()
        .route("/drafts", post(create_draft).get(list_drafts))
        .route("/drafts/{name}", get(get_draft).delete(delete_draft))
        .route("/drafts/{name}/validate", post(validate_draft))
        .route("/drafts/{name}/object-types", post(create_object_type))
        .route(
            "/drafts/{name}/object-types/{id}",
            patch(replace_object_type).delete(delete_object_type),
        )
        .route(
            "/drafts/{name}/object-types/{id}/properties",
            post(create_property_type),
        )
        .route(
            "/drafts/{name}/object-types/{id}/properties/{property_id}",
            patch(replace_property_type).delete(delete_property_type),
        )
        .route("/drafts/{name}/link-types", post(create_link_type))
        .route(
            "/drafts/{name}/link-types/{id}",
            patch(replace_link_type).delete(delete_link_type),
        )
        .route("/drafts/{name}/publish", post(publish_draft))
        .route("/revisions", get(list_revisions))
        .route("/revisions/{id}", get(get_revision))
        .route("/tags/{name}", get(get_tag).put(put_tag).delete(delete_tag))
        .route("/graph", get(graph))
}

#[derive(Deserialize)]
struct CreateDraftRequest {
    name: String,
    schema: SchemaDocument,
}
#[derive(Deserialize)]
struct ObjectTypeRequest {
    name: String,
    #[serde(default)]
    description: String,
}
#[derive(Deserialize)]
struct PropertyTypeRequest {
    name: String,
    #[serde(default)]
    description: String,
    value_type: ValueType,
    required: bool,
}
#[derive(Deserialize)]
struct LinkTypeRequest {
    name: String,
    #[serde(default)]
    description: String,
    source_object_type_id: ObjectTypeId,
    target_object_type_id: ObjectTypeId,
    cardinality: Cardinality,
}
#[derive(Deserialize)]
struct TagRequest {
    revision_id: RevisionId,
}
#[derive(Deserialize)]
struct GraphQuery {
    draft: Option<String>,
    revision: Option<String>,
    tag: Option<String>,
}

#[derive(Serialize)]
struct DraftResponse {
    name: DraftName,
    schema: SchemaDocument,
    digest: RevisionId,
}
#[derive(Serialize)]
struct TagResponse {
    name: TagName,
    revision_id: RevisionId,
}
#[derive(Serialize)]
struct RevisionResponse {
    id: RevisionId,
    schema: SchemaDocument,
}
#[derive(Serialize)]
struct GraphResponse {
    schema_ref: SchemaRefResponse,
    nodes: Vec<GraphNodeResponse>,
    edges: Vec<GraphEdgeResponse>,
}
#[derive(Serialize)]
struct SchemaRefResponse {
    kind: &'static str,
    name: String,
}
#[derive(Serialize)]
struct GraphNodeResponse {
    id: ObjectTypeId,
    label: String,
    property_count: usize,
}
#[derive(Serialize)]
struct GraphEdgeResponse {
    id: wyse_ontology::LinkTypeId,
    label: String,
    source: ObjectTypeId,
    target: ObjectTypeId,
    cardinality: Cardinality,
}

impl From<Draft> for DraftResponse {
    fn from(value: Draft) -> Self {
        Self {
            name: value.name,
            schema: value.schema,
            digest: value.digest,
        }
    }
}

impl From<PublishedRevision> for RevisionResponse {
    fn from(value: PublishedRevision) -> Self {
        Self {
            id: value.id,
            schema: value.schema,
        }
    }
}

impl From<GraphProjection> for GraphResponse {
    fn from(value: GraphProjection) -> Self {
        Self {
            schema_ref: schema_ref_response(value.schema_ref),
            nodes: value
                .nodes
                .into_iter()
                .map(|node| GraphNodeResponse {
                    id: node.id,
                    label: node.label,
                    property_count: node.property_count,
                })
                .collect(),
            edges: value
                .edges
                .into_iter()
                .map(|edge| GraphEdgeResponse {
                    id: edge.id,
                    label: edge.label,
                    source: edge.source,
                    target: edge.target,
                    cardinality: edge.cardinality,
                })
                .collect(),
        }
    }
}

async fn create_draft(
    State(state): State<AppState>,
    Json(request): Json<CreateDraftRequest>,
) -> Result<Response, ApiError> {
    let draft = state
        .service
        .create_draft(parse_draft_name(request.name)?, request.schema)
        .await
        .map_err(|error| match error {
            wyse_ontology::OntologyError::DraftConflict { .. } => ApiError::Conflict,
            error => ApiError::Ontology(error),
        })?;
    Ok(draft_response(StatusCode::CREATED, draft))
}

async fn list_drafts(State(state): State<AppState>) -> Result<Json<Vec<DraftResponse>>, ApiError> {
    Ok(Json(
        state
            .service
            .list_drafts()
            .await?
            .into_iter()
            .map(DraftResponse::from)
            .collect(),
    ))
}

async fn get_draft(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Response, ApiError> {
    Ok(draft_response(
        StatusCode::OK,
        state.service.get_draft(&parse_draft_name(name)?).await?,
    ))
}

async fn delete_draft(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    state
        .service
        .delete_draft(&parse_draft_name(name)?, if_match(&headers)?)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn validate_draft(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Response, ApiError> {
    Ok(draft_response(
        StatusCode::OK,
        state
            .service
            .validate_draft(&parse_draft_name(name)?)
            .await?,
    ))
}

async fn create_object_type(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ObjectTypeRequest>,
) -> Result<Response, ApiError> {
    let draft = state
        .service
        .add_object_type(
            &parse_draft_name(name)?,
            if_match(&headers)?,
            request.name,
            request.description,
        )
        .await?;
    Ok(draft_response(StatusCode::OK, draft))
}

async fn replace_object_type(
    State(state): State<AppState>,
    Path((name, id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<ObjectTypeRequest>,
) -> Result<Response, ApiError> {
    let draft = state
        .service
        .replace_object_type(
            &parse_draft_name(name)?,
            if_match(&headers)?,
            parse_id(&id)?,
            request.name,
            request.description,
        )
        .await?;
    Ok(draft_response(StatusCode::OK, draft))
}

async fn delete_object_type(
    State(state): State<AppState>,
    Path((name, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let draft = state
        .service
        .delete_object_type(
            &parse_draft_name(name)?,
            if_match(&headers)?,
            parse_id(&id)?,
        )
        .await?;
    Ok(draft_response(StatusCode::OK, draft))
}

async fn create_property_type(
    State(state): State<AppState>,
    Path((name, id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<PropertyTypeRequest>,
) -> Result<Response, ApiError> {
    let draft = state
        .service
        .add_property_type(
            &parse_draft_name(name)?,
            if_match(&headers)?,
            parse_id(&id)?,
            request.name,
            request.description,
            request.value_type,
            request.required,
        )
        .await?;
    Ok(draft_response(StatusCode::OK, draft))
}

async fn replace_property_type(
    State(state): State<AppState>,
    Path((name, id, property_id)): Path<(String, String, String)>,
    headers: HeaderMap,
    Json(request): Json<PropertyTypeRequest>,
) -> Result<Response, ApiError> {
    let draft = state
        .service
        .replace_property_type(
            &parse_draft_name(name)?,
            if_match(&headers)?,
            parse_id(&id)?,
            parse_id(&property_id)?,
            request.name,
            request.description,
            request.value_type,
            request.required,
        )
        .await?;
    Ok(draft_response(StatusCode::OK, draft))
}

async fn delete_property_type(
    State(state): State<AppState>,
    Path((name, id, property_id)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let draft = state
        .service
        .delete_property_type(
            &parse_draft_name(name)?,
            if_match(&headers)?,
            parse_id(&id)?,
            parse_id(&property_id)?,
        )
        .await?;
    Ok(draft_response(StatusCode::OK, draft))
}

async fn create_link_type(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(request): Json<LinkTypeRequest>,
) -> Result<Response, ApiError> {
    let draft = state
        .service
        .add_link_type(
            &parse_draft_name(name)?,
            if_match(&headers)?,
            request.name,
            request.description,
            request.source_object_type_id,
            request.target_object_type_id,
            request.cardinality,
        )
        .await?;
    Ok(draft_response(StatusCode::OK, draft))
}

async fn replace_link_type(
    State(state): State<AppState>,
    Path((name, id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<LinkTypeRequest>,
) -> Result<Response, ApiError> {
    let draft = state
        .service
        .replace_link_type(
            &parse_draft_name(name)?,
            if_match(&headers)?,
            parse_id(&id)?,
            request.name,
            request.description,
            request.source_object_type_id,
            request.target_object_type_id,
            request.cardinality,
        )
        .await?;
    Ok(draft_response(StatusCode::OK, draft))
}

async fn delete_link_type(
    State(state): State<AppState>,
    Path((name, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let draft = state
        .service
        .delete_link_type(
            &parse_draft_name(name)?,
            if_match(&headers)?,
            parse_id(&id)?,
        )
        .await?;
    Ok(draft_response(StatusCode::OK, draft))
}

async fn publish_draft(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<(StatusCode, Json<RevisionResponse>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(
            state
                .service
                .publish(&parse_draft_name(name)?)
                .await?
                .into(),
        ),
    ))
}

async fn list_revisions(
    State(state): State<AppState>,
) -> Result<Json<Vec<RevisionResponse>>, ApiError> {
    Ok(Json(
        state
            .service
            .list_revisions()
            .await?
            .into_iter()
            .map(RevisionResponse::from)
            .collect(),
    ))
}

async fn get_revision(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RevisionResponse>, ApiError> {
    Ok(Json(
        state
            .service
            .get_revision(&parse_revision_id(id)?)
            .await?
            .into(),
    ))
}

async fn get_tag(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<TagResponse>, ApiError> {
    let name = parse_tag_name(name)?;
    Ok(Json(TagResponse {
        revision_id: state.service.get_tag(&name).await?,
        name,
    }))
}

async fn put_tag(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(request): Json<TagRequest>,
) -> Result<StatusCode, ApiError> {
    state
        .service
        .put_tag(&parse_tag_name(name)?, &request.revision_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_tag(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.service.delete_tag(&parse_tag_name(name)?).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn graph(
    State(state): State<AppState>,
    Query(query): Query<GraphQuery>,
) -> Result<Json<GraphResponse>, ApiError> {
    Ok(Json(state.service.graph(schema_ref(query)?).await?.into()))
}

fn draft_response(status: StatusCode, draft: Draft) -> Response {
    let mut response = (status, Json(DraftResponse::from(draft.clone()))).into_response();
    let etag = HeaderValue::from_str(&format!("\"{}\"", draft.digest))
        .expect("revision digest is a valid HTTP header value");
    response.headers_mut().insert(header::ETAG, etag);
    response
}

fn if_match(headers: &HeaderMap) -> Result<RevisionId, ApiError> {
    let value = headers
        .get(header::IF_MATCH)
        .ok_or_else(|| ApiError::BadRequest("missing If-Match".to_owned()))?;
    let text = value
        .to_str()
        .map_err(|_| ApiError::BadRequest("invalid If-Match".to_owned()))?;
    parse_revision_id(text.trim_matches('"').to_owned())
}

fn schema_ref(query: GraphQuery) -> Result<SchemaRef, ApiError> {
    let count = [
        query.draft.is_some(),
        query.revision.is_some(),
        query.tag.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    if count != 1 {
        return Err(ApiError::BadRequest(
            "exactly one schema reference is required".to_owned(),
        ));
    }
    if let Some(name) = query.draft {
        return Ok(SchemaRef::Draft(parse_draft_name(name)?));
    }
    if let Some(id) = query.revision {
        return Ok(SchemaRef::Revision(parse_revision_id(id)?));
    }
    Ok(SchemaRef::Tag(parse_tag_name(
        query.tag.expect("one schema reference exists"),
    )?))
}

fn schema_ref_response(schema_ref: SchemaRef) -> SchemaRefResponse {
    match schema_ref {
        SchemaRef::Draft(name) => SchemaRefResponse {
            kind: "draft",
            name: name.to_string(),
        },
        SchemaRef::Revision(id) => SchemaRefResponse {
            kind: "revision",
            name: id.to_string(),
        },
        SchemaRef::Tag(name) => SchemaRefResponse {
            kind: "tag",
            name: name.to_string(),
        },
    }
}

fn parse_draft_name(value: String) -> Result<DraftName, ApiError> {
    value.try_into().map_err(ApiError::from)
}
fn parse_tag_name(value: String) -> Result<TagName, ApiError> {
    value.try_into().map_err(ApiError::from)
}
fn parse_revision_id(value: String) -> Result<RevisionId, ApiError> {
    value.try_into().map_err(ApiError::from)
}
fn parse_id<T: FromStr>(value: &str) -> Result<T, ApiError> {
    value
        .parse()
        .map_err(|_| ApiError::BadRequest("invalid UUID".to_owned()))
}
