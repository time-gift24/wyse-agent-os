//! Object and link instance HTTP routes.

use std::str::FromStr;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use wyse_ontology::{
    CreateLink, CreateObject, LinkId, LinkRecord, LinkTypeId, ObjectId, ObjectRecord, ObjectTypeId,
    Page, ReplaceLink, ReplaceObject, SchemaRef,
};

use crate::{AppState, error::ApiError};

/// Creates the `/v1` object and link routes.
pub fn instance_routes() -> Router<AppState> {
    Router::new()
        .route("/objects", post(create_object).get(page_objects))
        .route(
            "/objects/{id}",
            get(get_object).patch(replace_object).delete(delete_object),
        )
        .route("/links", post(create_link).get(page_links))
        .route(
            "/links/{id}",
            get(get_link).patch(replace_link).delete(delete_link),
        )
}

#[derive(Deserialize)]
struct CreateObjectRequest {
    schema_ref: SchemaRef,
    object_type_id: ObjectTypeId,
    values: Map<String, Value>,
}

#[derive(Deserialize)]
struct ReplaceObjectRequest {
    schema_ref: SchemaRef,
    version: u64,
    values: Map<String, Value>,
}

#[derive(Deserialize)]
struct ObjectQuery {
    draft: Option<String>,
    revision: Option<String>,
    tag: Option<String>,
    object_type_id: Option<ObjectTypeId>,
    after: Option<String>,
    limit: Option<u32>,
    version: Option<u64>,
    force: Option<bool>,
}

#[derive(Deserialize)]
struct CreateLinkRequest {
    schema_ref: SchemaRef,
    link_type_id: LinkTypeId,
    source_object_id: ObjectId,
    target_object_id: ObjectId,
}

#[derive(Deserialize)]
struct ReplaceLinkRequest {
    schema_ref: SchemaRef,
    version: u64,
    source_object_id: ObjectId,
    target_object_id: ObjectId,
}

#[derive(Deserialize)]
struct LinkQuery {
    draft: Option<String>,
    revision: Option<String>,
    tag: Option<String>,
    after: Option<String>,
    limit: Option<u32>,
    version: Option<u64>,
}

#[derive(Serialize)]
struct ObjectResponse {
    id: ObjectId,
    object_type_id: ObjectTypeId,
    values: Map<String, Value>,
    version: u64,
}

#[derive(Serialize)]
struct LinkResponse {
    id: LinkId,
    link_type_id: LinkTypeId,
    source_object_id: ObjectId,
    target_object_id: ObjectId,
    version: u64,
}

#[derive(Serialize)]
struct PageResponse<T> {
    items: Vec<T>,
    next_after: Option<String>,
}

impl From<ObjectRecord> for ObjectResponse {
    fn from(value: ObjectRecord) -> Self {
        Self {
            id: value.id,
            object_type_id: value.object_type_id,
            values: value.values,
            version: value.version,
        }
    }
}

impl From<LinkRecord> for LinkResponse {
    fn from(value: LinkRecord) -> Self {
        Self {
            id: value.id,
            link_type_id: value.link_type_id,
            source_object_id: value.source_object_id,
            target_object_id: value.target_object_id,
            version: value.version,
        }
    }
}

async fn create_object(
    State(state): State<AppState>,
    Json(request): Json<CreateObjectRequest>,
) -> Result<(StatusCode, Json<ObjectResponse>), ApiError> {
    let object = state
        .service
        .create_object(CreateObject {
            schema_ref: request.schema_ref,
            object_type_id: request.object_type_id,
            values: request.values,
        })
        .await?;
    Ok((StatusCode::CREATED, Json(object.into())))
}

async fn page_objects(
    State(state): State<AppState>,
    Query(query): Query<ObjectQuery>,
) -> Result<Json<PageResponse<ObjectResponse>>, ApiError> {
    let object_type_id = query
        .object_type_id
        .ok_or_else(|| ApiError::BadRequest("object_type_id is required".to_owned()))?;
    let page = state
        .service
        .page_objects(
            schema_ref(query.draft, query.revision, query.tag)?,
            object_type_id,
            parse_optional_id(query.after)?,
            query.limit.unwrap_or(50),
        )
        .await?;
    Ok(Json(object_page_response(page)))
}

async fn get_object(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ObjectQuery>,
) -> Result<Json<ObjectResponse>, ApiError> {
    Ok(Json(
        state
            .service
            .get_object(
                schema_ref(query.draft, query.revision, query.tag)?,
                parse_id(&id)?,
            )
            .await?
            .into(),
    ))
}

async fn replace_object(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ReplaceObjectRequest>,
) -> Result<Json<ObjectResponse>, ApiError> {
    Ok(Json(
        state
            .service
            .replace_object(
                parse_id(&id)?,
                ReplaceObject {
                    schema_ref: request.schema_ref,
                    version: request.version,
                    values: request.values,
                },
            )
            .await?
            .into(),
    ))
}

async fn delete_object(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ObjectQuery>,
) -> Result<StatusCode, ApiError> {
    let version = query
        .version
        .ok_or_else(|| ApiError::BadRequest("version is required".to_owned()))?;
    state
        .service
        .delete_object(
            schema_ref(query.draft, query.revision, query.tag)?,
            parse_id(&id)?,
            version,
            query.force.unwrap_or(false),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_link(
    State(state): State<AppState>,
    Json(request): Json<CreateLinkRequest>,
) -> Result<(StatusCode, Json<LinkResponse>), ApiError> {
    let link = state
        .service
        .create_link(CreateLink {
            schema_ref: request.schema_ref,
            link_type_id: request.link_type_id,
            source_object_id: request.source_object_id,
            target_object_id: request.target_object_id,
        })
        .await?;
    Ok((StatusCode::CREATED, Json(link.into())))
}

async fn page_links(
    State(state): State<AppState>,
    Query(query): Query<LinkQuery>,
) -> Result<Json<PageResponse<LinkResponse>>, ApiError> {
    let page = state
        .service
        .page_links(
            schema_ref(query.draft, query.revision, query.tag)?,
            parse_optional_id(query.after)?,
            query.limit.unwrap_or(50),
        )
        .await?;
    Ok(Json(link_page_response(page)))
}

async fn get_link(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<LinkQuery>,
) -> Result<Json<LinkResponse>, ApiError> {
    Ok(Json(
        state
            .service
            .get_link(
                schema_ref(query.draft, query.revision, query.tag)?,
                parse_id(&id)?,
            )
            .await?
            .into(),
    ))
}

async fn replace_link(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ReplaceLinkRequest>,
) -> Result<Json<LinkResponse>, ApiError> {
    Ok(Json(
        state
            .service
            .replace_link(
                parse_id(&id)?,
                ReplaceLink {
                    schema_ref: request.schema_ref,
                    version: request.version,
                    source_object_id: request.source_object_id,
                    target_object_id: request.target_object_id,
                },
            )
            .await?
            .into(),
    ))
}

async fn delete_link(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<LinkQuery>,
) -> Result<StatusCode, ApiError> {
    let version = query
        .version
        .ok_or_else(|| ApiError::BadRequest("version is required".to_owned()))?;
    state
        .service
        .delete_link(
            schema_ref(query.draft, query.revision, query.tag)?,
            parse_id(&id)?,
            version,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn object_page_response(page: Page<ObjectRecord>) -> PageResponse<ObjectResponse> {
    PageResponse {
        items: page.items.into_iter().map(ObjectResponse::from).collect(),
        next_after: page.next_after.map(|id| id.to_string()),
    }
}

fn link_page_response(page: Page<LinkRecord>) -> PageResponse<LinkResponse> {
    PageResponse {
        items: page.items.into_iter().map(LinkResponse::from).collect(),
        next_after: page.next_after.map(|id| id.to_string()),
    }
}

fn schema_ref(
    draft: Option<String>,
    revision: Option<String>,
    tag: Option<String>,
) -> Result<SchemaRef, ApiError> {
    let count = [draft.is_some(), revision.is_some(), tag.is_some()]
        .into_iter()
        .filter(|present| *present)
        .count();
    if count != 1 {
        return Err(ApiError::BadRequest(
            "exactly one schema reference is required".to_owned(),
        ));
    }
    if let Some(name) = draft {
        return Ok(SchemaRef::Draft(name.try_into().map_err(ApiError::from)?));
    }
    if let Some(id) = revision {
        return Ok(SchemaRef::Revision(id.try_into().map_err(ApiError::from)?));
    }
    Ok(SchemaRef::Tag(
        tag.expect("one schema reference exists")
            .try_into()
            .map_err(ApiError::from)?,
    ))
}

fn parse_id<T: FromStr>(value: &str) -> Result<T, ApiError> {
    value
        .parse()
        .map_err(|_| ApiError::BadRequest("invalid UUID".to_owned()))
}

fn parse_optional_id<T: FromStr>(value: Option<String>) -> Result<Option<T>, ApiError> {
    value.as_deref().map(parse_id).transpose()
}
