use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{HeaderValue, Request, StatusCode, header},
};
use serde::de::DeserializeOwned;
use tower::ServiceExt;
use wyse_filesystem::{
    CasExpectation, DirEntry, Entry, FileMetadata, Filesystem, FilesystemError, RecordVersion,
    VersionedEntry, VirtualPath,
};
use wyse_ontology::{
    Cardinality, DraftName, FilesystemDraftStore, LinkCardinalityConstraint, LinkId, LinkRecord,
    LinkType, LinkTypeId, NewLinkRecord, NewObjectRecord, ObjectId, ObjectRecord, ObjectType,
    ObjectTypeId, OntologyError, OntologyRepository, Page, PropertyType, PropertyTypeId,
    PublishedRevision, RevisionId, SchemaDocument, SchemaValidationSnapshot, TagName, ValueType,
};
use wyse_ontology_api::router;

#[tokio::test]
async fn graph_route_returns_schema_nodes_and_edges() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(test_service_with_online_schema().await?);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/ontology/graph?tag=online")
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await?;
    assert_eq!(body["nodes"].as_array().map(Vec::len), Some(2));
    assert_eq!(body["edges"].as_array().map(Vec::len), Some(1));
    Ok(())
}

#[tokio::test]
async fn create_draft_returns_an_etag() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(test_service_with_online_schema().await?);
    let response = app
        .oneshot(Request::builder().method("POST").uri("/v1/ontology/drafts").header("content-type", "application/json").body(Body::from(r#"{"name":"experiment","schema":{"schema_version":1,"object_types":[],"link_types":[]}}"#))?)
        .await?;

    assert_eq!(response.status(), StatusCode::CREATED);
    assert!(response.headers().contains_key("etag"));
    Ok(())
}

#[tokio::test]
async fn creating_an_existing_draft_returns_conflict() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(test_service_with_online_schema().await?);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ontology/drafts")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"name":"main","schema":{"schema_version":1,"object_types":[],"link_types":[]}}"#,
                ))?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    Ok(())
}

#[tokio::test]
async fn current_if_match_allows_a_schema_mutation() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(test_service_with_online_schema().await?);
    let draft = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/ontology/drafts/main")
                .body(Body::empty())?,
        )
        .await?;
    let etag = draft.headers()["etag"].clone();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ontology/drafts/main/object-types")
                .header("content-type", "application/json")
                .header("if-match", etag)
                .body(Body::from(r#"{"name":"Project"}"#))?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().contains_key("etag"));
    let body: serde_json::Value = decode_json(response).await?;
    assert_eq!(
        body["schema"]["object_types"].as_array().map(Vec::len),
        Some(3)
    );
    Ok(())
}

#[tokio::test]
async fn deleting_a_draft_with_a_stale_if_match_returns_precondition_failed()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(test_service_with_online_schema().await?);
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/v1/ontology/drafts/main")
                .header(
                    "if-match",
                    "\"0000000000000000000000000000000000000000000000000000000000000000\"",
                )
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::PRECONDITION_FAILED);
    Ok(())
}

#[tokio::test]
async fn deleting_the_online_tag_returns_conflict() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(test_service_with_online_schema().await?);
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/v1/ontology/tags/online")
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    Ok(())
}

#[tokio::test]
async fn validate_returns_unprocessable_entity_for_an_invalid_static_schema()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(test_service_with_online_schema().await?);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ontology/drafts/bad/validate")
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    Ok(())
}

#[tokio::test]
async fn object_list_requires_a_schema_reference_and_returns_a_page()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(test_service_with_online_schema().await?);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/objects?tag=online&object_type_id={}&limit=1",
                    test_person_type_id()
                ))
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = decode_json(response).await?;
    assert!(body["items"].is_array());
    Ok(())
}

#[tokio::test]
async fn object_and_link_routes_enforce_versions_values_cardinality_and_force_delete()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(test_service_with_online_schema().await?);
    let person = create_object(&app, test_person_type_id(), "Ada").await?;
    let company = create_object(&app, ObjectTypeId::from(uuid::Uuid::from_u128(2)), "Wyse").await?;

    let created_link = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/v1/links",
            serde_json::json!({
                "schema_ref": {"tag": "online"},
                "link_type_id": test_link_type_id(),
                "source_object_id": person["id"],
                "target_object_id": company["id"]
            }),
        )?)
        .await?;
    assert_eq!(created_link.status(), StatusCode::CREATED);
    let link: serde_json::Value = decode_json(created_link).await?;

    let page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/objects?tag=online&object_type_id={}&limit=1",
                    test_person_type_id()
                ))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(page.status(), StatusCode::OK);
    let page: serde_json::Value = decode_json(page).await?;
    assert_eq!(page["items"].as_array().map(Vec::len), Some(1));

    let referenced_delete = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/v1/objects/{}?tag=online&version=1",
                    json_id(&person)
                ))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(referenced_delete.status(), StatusCode::CONFLICT);

    let stale_object = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            format!("/v1/objects/{}", json_id(&person)),
            serde_json::json!({"schema_ref":{"tag":"online"},"version":0,"values":{"name":"Ada"}}),
        )?)
        .await?;
    assert_eq!(stale_object.status(), StatusCode::PRECONDITION_FAILED);

    let invalid_values = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            format!("/v1/objects/{}", json_id(&person)),
            serde_json::json!({"schema_ref":{"tag":"online"},"version":1,"values":{"unknown":"value"}}),
        )?)
        .await?;
    assert_eq!(invalid_values.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let second_person = create_object(&app, test_person_type_id(), "Grace").await?;
    let cardinality_conflict = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/v1/links",
            serde_json::json!({
                "schema_ref":{"tag":"online"},
                "link_type_id":test_link_type_id(),
                "source_object_id":second_person["id"],
                "target_object_id":company["id"]
            }),
        )?)
        .await?;
    assert_eq!(cardinality_conflict.status(), StatusCode::CONFLICT);

    let stale_link = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            format!("/v1/links/{}", json_id(&link)),
            serde_json::json!({
                "schema_ref":{"tag":"online"},"version":0,
                "source_object_id":person["id"],"target_object_id":company["id"]
            }),
        )?)
        .await?;
    assert_eq!(stale_link.status(), StatusCode::PRECONDITION_FAILED);

    let forced_delete = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/v1/objects/{}?tag=online&version=1&force=true",
                    json_id(&person)
                ))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(forced_delete.status(), StatusCode::NO_CONTENT);
    let removed_link = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/links/{}?tag=online", json_id(&link)))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(removed_link.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn http_flow_creates_publishes_uses_and_restores_an_ontology()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(test_service_without_schema());
    let created = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/v1/ontology/drafts",
            serde_json::json!({
                "name":"flow",
                "schema":{"schema_version":1,"object_types":[],"link_types":[]}
            }),
        )?)
        .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let mut if_match = response_etag(&created)?;

    let person_type = app
        .clone()
        .oneshot(json_if_match_request(
            "POST",
            "/v1/ontology/drafts/flow/object-types",
            &if_match,
            serde_json::json!({"name":"Person"}),
        )?)
        .await?;
    assert_eq!(person_type.status(), StatusCode::OK);
    if_match = response_etag(&person_type)?;
    let person_type: serde_json::Value = decode_json(person_type).await?;
    let person_type_id = person_type["schema"]["object_types"][0]["id"].clone();

    let company_type = app
        .clone()
        .oneshot(json_if_match_request(
            "POST",
            "/v1/ontology/drafts/flow/object-types",
            &if_match,
            serde_json::json!({"name":"Company"}),
        )?)
        .await?;
    assert_eq!(company_type.status(), StatusCode::OK);
    if_match = response_etag(&company_type)?;
    let company_type: serde_json::Value = decode_json(company_type).await?;
    let company_type_id = company_type["schema"]["object_types"][1]["id"].clone();

    let link_type = app
        .clone()
        .oneshot(json_if_match_request(
            "POST",
            "/v1/ontology/drafts/flow/link-types",
            &if_match,
            serde_json::json!({
                "name":"works_for",
                "source_object_type_id":person_type_id,
                "target_object_type_id":company_type_id,
                "cardinality":"one_to_many"
            }),
        )?)
        .await?;
    assert_eq!(link_type.status(), StatusCode::OK);
    let link_type: serde_json::Value = decode_json(link_type).await?;
    let link_type_id = link_type["schema"]["link_types"][0]["id"].clone();

    let validated = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ontology/drafts/flow/validate")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(validated.status(), StatusCode::OK);

    let published = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ontology/drafts/flow/publish")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(published.status(), StatusCode::CREATED);
    let published: serde_json::Value = decode_json(published).await?;
    let revision_id = published["id"].clone();

    let tagged = app
        .clone()
        .oneshot(json_request(
            "PUT",
            "/v1/ontology/tags/online",
            serde_json::json!({"revision_id":revision_id}),
        )?)
        .await?;
    assert_eq!(tagged.status(), StatusCode::NO_CONTENT);

    let graph = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/ontology/graph?tag=online")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(graph.status(), StatusCode::OK);
    let graph: serde_json::Value = decode_json(graph).await?;
    assert_eq!(graph["nodes"].as_array().map(Vec::len), Some(2));
    assert_eq!(graph["edges"].as_array().map(Vec::len), Some(1));

    let first_person = create_object_with_type(&app, &person_type_id).await?;
    let second_person = create_object_with_type(&app, &person_type_id).await?;
    let company = create_object_with_type(&app, &company_type_id).await?;
    let first_link = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/v1/links",
            serde_json::json!({
                "schema_ref":{"tag":"online"},
                "link_type_id":link_type_id,
                "source_object_id":first_person["id"],
                "target_object_id":company["id"]
            }),
        )?)
        .await?;
    assert_eq!(first_link.status(), StatusCode::CREATED);

    let page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/objects?tag=online&object_type_id={}&limit=1",
                    person_type_id.as_str().expect("type id is a string")
                ))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(page.status(), StatusCode::OK);
    let page: serde_json::Value = decode_json(page).await?;
    assert_eq!(page["items"].as_array().map(Vec::len), Some(1));

    let cardinality_conflict = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/v1/links",
            serde_json::json!({
                "schema_ref":{"tag":"online"},
                "link_type_id":link_type_id,
                "source_object_id":second_person["id"],
                "target_object_id":company["id"]
            }),
        )?)
        .await?;
    assert_eq!(cardinality_conflict.status(), StatusCode::CONFLICT);

    let forced_delete = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/v1/objects/{}?tag=online&version=1&force=true",
                    json_id(&first_person)
                ))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(forced_delete.status(), StatusCode::NO_CONTENT);

    let revision = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/ontology/revisions/{}",
                    revision_id.as_str().expect("revision id is a string")
                ))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(revision.status(), StatusCode::OK);
    let revision: serde_json::Value = decode_json(revision).await?;

    let restored = app
        .oneshot(json_request(
            "POST",
            "/v1/ontology/drafts",
            serde_json::json!({"name":"restored","schema":revision["schema"]}),
        )?)
        .await?;
    assert_eq!(restored.status(), StatusCode::CREATED);
    Ok(())
}

#[tokio::test]
async fn memory_filesystem_delete_requires_a_matching_cas_version()
-> Result<(), Box<dyn std::error::Error>> {
    let filesystem = MemoryFilesystem::default();
    let path = VirtualPath::try_from("/ontology/drafts/cas.json")?;
    assert!(matches!(
        filesystem.delete(&path, CasExpectation::Any).await,
        Err(FilesystemError::VersionMismatch { .. })
    ));

    let version = filesystem
        .put(&path, Entry::new(Vec::new()), CasExpectation::Absent)
        .await?;
    assert!(matches!(
        filesystem
            .delete(
                &path,
                CasExpectation::Version(RecordVersion::from_backend(999)),
            )
            .await,
        Err(FilesystemError::VersionMismatch { .. })
    ));
    assert!(filesystem.get(&path).await?.is_some());
    filesystem
        .delete(&path, CasExpectation::Version(version))
        .await?;
    assert!(filesystem.get(&path).await?.is_none());
    Ok(())
}

async fn create_object(
    app: &axum::Router,
    object_type_id: ObjectTypeId,
    name: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/v1/objects",
            serde_json::json!({
                "schema_ref":{"tag":"online"},
                "object_type_id":object_type_id,
                "values":{"name":name}
            }),
        )?)
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    decode_json(response).await
}

async fn create_object_with_type(
    app: &axum::Router,
    object_type_id: &serde_json::Value,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/v1/objects",
            serde_json::json!({
                "schema_ref":{"tag":"online"},
                "object_type_id":object_type_id,
                "values":{}
            }),
        )?)
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    decode_json(response).await
}

fn json_request(
    method: &str,
    uri: impl AsRef<str>,
    value: serde_json::Value,
) -> Result<Request<Body>, axum::http::Error> {
    Request::builder()
        .method(method)
        .uri(uri.as_ref())
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&value).expect("JSON value serializes"),
        ))
}

fn json_if_match_request(
    method: &str,
    uri: impl AsRef<str>,
    etag: &HeaderValue,
    value: serde_json::Value,
) -> Result<Request<Body>, axum::http::Error> {
    let mut request = json_request(method, uri, value)?;
    request.headers_mut().insert(header::IF_MATCH, etag.clone());
    Ok(request)
}

fn response_etag(
    response: &axum::response::Response,
) -> Result<HeaderValue, Box<dyn std::error::Error>> {
    response
        .headers()
        .get(header::ETAG)
        .cloned()
        .ok_or_else(|| "response is missing ETag".into())
}

fn json_id(value: &serde_json::Value) -> &str {
    value["id"].as_str().expect("response has a string id")
}

async fn decode_json<T: DeserializeOwned>(
    response: axum::response::Response,
) -> Result<T, Box<dyn std::error::Error>> {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn test_service_with_online_schema()
-> Result<Arc<wyse_ontology::OntologyService>, Box<dyn std::error::Error>> {
    let person = test_person_type_id();
    let company = ObjectTypeId::from(uuid::Uuid::from_u128(2));
    let schema = SchemaDocument {
        schema_version: 1,
        object_types: vec![
            object_type(person, "Person"),
            object_type(company, "Company"),
        ],
        link_types: vec![LinkType::new(
            test_link_type_id(),
            "works_for".to_owned(),
            person,
            company,
            Cardinality::OneToMany,
        )],
    };
    let filesystem = Arc::new(MemoryFilesystem::default());
    let drafts = FilesystemDraftStore::new(filesystem.clone());
    drafts
        .create(DraftName::try_from("main".to_owned())?, schema.clone())
        .await?;
    let repository = Arc::new(MemoryRepository::default());
    let service = Arc::new(wyse_ontology::OntologyService::new(
        drafts,
        repository.clone(),
    ));
    let revision = service
        .publish(&DraftName::try_from("main".to_owned())?)
        .await?;
    service.put_tag(&TagName::online(), &revision.id).await?;
    let invalid = SchemaDocument {
        schema_version: 0,
        object_types: Vec::new(),
        link_types: Vec::new(),
    };
    let path = VirtualPath::try_from("/ontology/drafts/bad.json")?;
    filesystem
        .put(
            &path,
            Entry::new(serde_json::to_vec(&invalid)?),
            CasExpectation::Absent,
        )
        .await?;
    Ok(service)
}

fn test_service_without_schema() -> Arc<wyse_ontology::OntologyService> {
    Arc::new(wyse_ontology::OntologyService::new(
        FilesystemDraftStore::new(Arc::new(MemoryFilesystem::default())),
        Arc::new(MemoryRepository::default()),
    ))
}

fn test_person_type_id() -> ObjectTypeId {
    ObjectTypeId::from(uuid::Uuid::from_u128(1))
}

fn test_link_type_id() -> LinkTypeId {
    LinkTypeId::from(uuid::Uuid::from_u128(3))
}

fn object_type(id: ObjectTypeId, name: &str) -> ObjectType {
    ObjectType {
        id,
        name: name.to_owned(),
        description: String::new(),
        properties: vec![PropertyType {
            id: PropertyTypeId::new(),
            name: "name".to_owned(),
            description: String::new(),
            value_type: ValueType::String,
            required: true,
        }],
    }
}

#[derive(Default)]
struct MemoryFilesystem {
    entries: Mutex<BTreeMap<VirtualPath, VersionedEntry>>,
    next: Mutex<u64>,
}

#[async_trait]
impl Filesystem for MemoryFilesystem {
    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        Ok(self
            .entries
            .lock()
            .map_err(|_| FilesystemError::UnsupportedCas)?
            .get(path)
            .cloned())
    }
    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| FilesystemError::UnsupportedCas)?;
        let matches = match cas {
            CasExpectation::Absent => !entries.contains_key(path),
            CasExpectation::Version(expected) => entries
                .get(path)
                .is_some_and(|current| current.version == expected),
            CasExpectation::Any => true,
        };
        if !matches {
            return Err(FilesystemError::VersionMismatch { path: path.clone() });
        }
        let mut next = self
            .next
            .lock()
            .map_err(|_| FilesystemError::UnsupportedCas)?;
        *next += 1;
        let version = RecordVersion::from_backend(*next);
        entries.insert(path.clone(), VersionedEntry { entry, version });
        Ok(version)
    }
    async fn delete(&self, path: &VirtualPath, cas: CasExpectation) -> Result<(), FilesystemError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| FilesystemError::UnsupportedCas)?;
        let matches = entries.get(path).is_some_and(|current| match cas {
            CasExpectation::Absent => false,
            CasExpectation::Version(expected) => current.version == expected,
            CasExpectation::Any => true,
        });
        if !matches {
            return Err(FilesystemError::VersionMismatch { path: path.clone() });
        }
        entries.remove(path);
        Ok(())
    }
    async fn read_file(&self, _: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        Err(FilesystemError::UnsupportedCas)
    }
    async fn write_file(&self, _: &VirtualPath, _: Vec<u8>) -> Result<(), FilesystemError> {
        Err(FilesystemError::UnsupportedCas)
    }
    async fn list_dir(&self, _: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        Ok(Vec::new())
    }
    async fn metadata(&self, _: &VirtualPath) -> Result<FileMetadata, FilesystemError> {
        Err(FilesystemError::UnsupportedCas)
    }
    async fn create_dir(&self, _: &VirtualPath) -> Result<(), FilesystemError> {
        Err(FilesystemError::UnsupportedCas)
    }
    async fn remove_file(&self, _: &VirtualPath) -> Result<(), FilesystemError> {
        Err(FilesystemError::UnsupportedCas)
    }
    async fn remove_dir(&self, _: &VirtualPath) -> Result<(), FilesystemError> {
        Err(FilesystemError::UnsupportedCas)
    }
}

#[derive(Default)]
struct MemoryRepository {
    instances: Mutex<MemoryInstances>,
    revisions: Mutex<BTreeMap<RevisionId, PublishedRevision>>,
    tags: Mutex<BTreeMap<TagName, RevisionId>>,
}

#[derive(Default)]
struct MemoryInstances {
    objects: BTreeMap<ObjectId, ObjectRecord>,
    links: BTreeMap<LinkId, LinkRecord>,
}

#[async_trait]
impl OntologyRepository for MemoryRepository {
    async fn insert_revision(&self, revision: PublishedRevision) -> Result<(), OntologyError> {
        self.revisions
            .lock()
            .map_err(|_| repository_error())?
            .insert(revision.id.clone(), revision);
        Ok(())
    }
    async fn publish_revision(&self, revision: PublishedRevision) -> Result<(), OntologyError> {
        wyse_ontology::validate_published_revision(&revision)?;
        let instances = self.instances.lock().map_err(|_| repository_error())?;
        let objects = instances.objects.values().cloned().collect::<Vec<_>>();
        let links = instances.links.values().cloned().collect::<Vec<_>>();
        drop(instances);
        wyse_ontology::validate_schema_instances(&revision.schema, &objects, &links)?;
        self.revisions
            .lock()
            .map_err(|_| repository_error())?
            .insert(revision.id.clone(), revision);
        Ok(())
    }
    async fn get_revision(
        &self,
        id: &RevisionId,
    ) -> Result<Option<PublishedRevision>, OntologyError> {
        Ok(self
            .revisions
            .lock()
            .map_err(|_| repository_error())?
            .get(id)
            .cloned())
    }
    async fn list_revisions(&self) -> Result<Vec<PublishedRevision>, OntologyError> {
        Ok(self
            .revisions
            .lock()
            .map_err(|_| repository_error())?
            .values()
            .cloned()
            .collect())
    }
    async fn put_tag(&self, name: &TagName, id: &RevisionId) -> Result<(), OntologyError> {
        self.tags
            .lock()
            .map_err(|_| repository_error())?
            .insert(name.clone(), id.clone());
        Ok(())
    }
    async fn get_tag(&self, name: &TagName) -> Result<Option<RevisionId>, OntologyError> {
        Ok(self
            .tags
            .lock()
            .map_err(|_| repository_error())?
            .get(name)
            .cloned())
    }
    async fn delete_tag(&self, name: &TagName) -> Result<(), OntologyError> {
        self.tags
            .lock()
            .map_err(|_| repository_error())?
            .remove(name);
        Ok(())
    }
    async fn schema_validation_snapshot(&self) -> Result<SchemaValidationSnapshot, OntologyError> {
        let instances = self.instances.lock().map_err(|_| repository_error())?;
        Ok(SchemaValidationSnapshot {
            objects: instances.objects.values().cloned().collect(),
            links: instances.links.values().cloned().collect(),
        })
    }
    async fn create_object(&self, object: NewObjectRecord) -> Result<ObjectRecord, OntologyError> {
        let record = ObjectRecord {
            id: object.id,
            object_type_id: object.object_type_id,
            values: object.values,
            version: 1,
        };
        self.instances
            .lock()
            .map_err(|_| repository_error())?
            .objects
            .insert(record.id, record.clone());
        Ok(record)
    }
    async fn get_object(&self, id: ObjectId) -> Result<Option<ObjectRecord>, OntologyError> {
        Ok(self
            .instances
            .lock()
            .map_err(|_| repository_error())?
            .objects
            .get(&id)
            .cloned())
    }
    async fn page_objects(
        &self,
        type_id: ObjectTypeId,
        after: Option<ObjectId>,
        limit: u32,
    ) -> Result<Page<ObjectRecord>, OntologyError> {
        let mut items = self
            .instances
            .lock()
            .map_err(|_| repository_error())?
            .objects
            .values()
            .filter(|object| object.object_type_id == type_id)
            .filter(|object| after.is_none_or(|after| object.id > after))
            .cloned()
            .collect::<Vec<_>>();
        let has_next = items.len() > limit as usize;
        items.truncate(limit as usize);
        let next_after = has_next.then(|| {
            items
                .last()
                .expect("non-empty page has a cursor")
                .id
                .as_uuid()
        });
        Ok(Page { items, next_after })
    }
    async fn replace_object(&self, object: ObjectRecord) -> Result<ObjectRecord, OntologyError> {
        let mut instances = self.instances.lock().map_err(|_| repository_error())?;
        let Some(current) = instances.objects.get(&object.id) else {
            return Err(OntologyError::ObjectMissing { id: object.id });
        };
        if current.version != object.version {
            return Err(OntologyError::ObjectVersionConflict { id: object.id });
        }
        let updated = ObjectRecord {
            version: object.version + 1,
            ..object
        };
        instances.objects.insert(updated.id, updated.clone());
        Ok(updated)
    }
    async fn delete_object(
        &self,
        id: ObjectId,
        version: u64,
        force: bool,
    ) -> Result<(), OntologyError> {
        let mut instances = self.instances.lock().map_err(|_| repository_error())?;
        let Some(current) = instances.objects.get(&id) else {
            return Err(OntologyError::ObjectMissing { id });
        };
        if current.version != version {
            return Err(OntologyError::ObjectVersionConflict { id });
        }
        let referenced = instances
            .links
            .values()
            .any(|link| link.source_object_id == id || link.target_object_id == id);
        if referenced && !force {
            return Err(OntologyError::ObjectReferenced { id });
        }
        if force {
            instances
                .links
                .retain(|_, link| link.source_object_id != id && link.target_object_id != id);
        }
        instances.objects.remove(&id);
        Ok(())
    }
    async fn create_link_with_cardinality(
        &self,
        link: NewLinkRecord,
        constraints: &[LinkCardinalityConstraint],
    ) -> Result<LinkRecord, OntologyError> {
        let record = LinkRecord {
            id: link.id,
            link_type_id: link.link_type_id,
            source_object_id: link.source_object_id,
            target_object_id: link.target_object_id,
            version: 1,
        };
        let mut instances = self.instances.lock().map_err(|_| repository_error())?;
        check_cardinality(
            instances
                .links
                .values()
                .filter(|existing| existing.link_type_id == record.link_type_id),
            &record,
            constraints,
        )?;
        instances.links.insert(record.id, record.clone());
        Ok(record)
    }
    async fn get_link(&self, id: LinkId) -> Result<Option<LinkRecord>, OntologyError> {
        Ok(self
            .instances
            .lock()
            .map_err(|_| repository_error())?
            .links
            .get(&id)
            .cloned())
    }
    async fn page_links(
        &self,
        after: Option<LinkId>,
        limit: u32,
    ) -> Result<Page<LinkRecord>, OntologyError> {
        let mut items = self
            .instances
            .lock()
            .map_err(|_| repository_error())?
            .links
            .values()
            .filter(|link| after.is_none_or(|after| link.id > after))
            .cloned()
            .collect::<Vec<_>>();
        let has_next = items.len() > limit as usize;
        items.truncate(limit as usize);
        let next_after = has_next.then(|| {
            items
                .last()
                .expect("non-empty page has a cursor")
                .id
                .as_uuid()
        });
        Ok(Page { items, next_after })
    }
    async fn replace_link_with_cardinality(
        &self,
        link: LinkRecord,
        constraints: &[LinkCardinalityConstraint],
    ) -> Result<LinkRecord, OntologyError> {
        let mut instances = self.instances.lock().map_err(|_| repository_error())?;
        let Some(current) = instances.links.get(&link.id) else {
            return Err(OntologyError::LinkMissing { id: link.id });
        };
        if current.version != link.version {
            return Err(OntologyError::LinkVersionConflict { id: link.id });
        }
        check_cardinality(
            instances.links.values().filter(|existing| {
                existing.link_type_id == link.link_type_id && existing.id != link.id
            }),
            &link,
            constraints,
        )?;
        let updated = LinkRecord {
            version: link.version + 1,
            ..link
        };
        instances.links.insert(updated.id, updated.clone());
        Ok(updated)
    }
    async fn delete_link(&self, id: LinkId, version: u64) -> Result<(), OntologyError> {
        let mut instances = self.instances.lock().map_err(|_| repository_error())?;
        let Some(current) = instances.links.get(&id) else {
            return Err(OntologyError::LinkMissing { id });
        };
        if current.version != version {
            return Err(OntologyError::LinkVersionConflict { id });
        }
        instances.links.remove(&id);
        Ok(())
    }
}

fn check_cardinality<'a>(
    links: impl Iterator<Item = &'a LinkRecord>,
    candidate: &LinkRecord,
    constraints: &[LinkCardinalityConstraint],
) -> Result<(), OntologyError> {
    let links = links.collect::<Vec<_>>();
    let source_count = links
        .iter()
        .filter(|link| link.source_object_id == candidate.source_object_id)
        .count();
    let target_count = links
        .iter()
        .filter(|link| link.target_object_id == candidate.target_object_id)
        .count();
    if constraints
        .iter()
        .all(|constraint| match constraint.cardinality {
            Cardinality::OneToOne => source_count == 0 && target_count == 0,
            Cardinality::OneToMany => target_count == 0,
            Cardinality::ManyToOne => source_count == 0,
            Cardinality::ManyToMany => true,
        })
    {
        Ok(())
    } else {
        Err(OntologyError::CardinalityConflict {
            link_type_id: candidate.link_type_id,
        })
    }
}

fn repository_error() -> OntologyError {
    OntologyError::Repository("test repository lock poisoned".into())
}
