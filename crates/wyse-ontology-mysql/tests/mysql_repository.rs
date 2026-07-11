use std::sync::Arc;

use serde_json::Map;
use tokio::{
    sync::Barrier,
    time::{Duration, timeout},
};
use wyse_ontology::{
    Cardinality, LinkCardinalityConstraint, LinkId, LinkRecord, LinkTypeId, NewLinkRecord,
    NewObjectRecord, ObjectId, ObjectType, ObjectTypeId, OntologyError, OntologyRepository,
    PublishedRevision, RevisionId, SchemaDocument, TagName, canonical_schema_bytes, revision_id,
};
use wyse_ontology_mysql::SqlxOntologyRepository;

fn published_revision() -> PublishedRevision {
    let schema = SchemaDocument {
        schema_version: 1,
        object_types: vec![ObjectType {
            id: ObjectTypeId::new(),
            name: "person".to_owned(),
            description: String::new(),
            properties: Vec::new(),
        }],
        link_types: Vec::new(),
    };
    let id = revision_id(&schema).expect("valid schema has a revision id");
    PublishedRevision { id, schema }
}

#[tokio::test]
#[ignore = "requires MySQL 8 started by the crate Makefile"]
async fn repository_persists_revision_and_online_tag() -> Result<(), Box<dyn std::error::Error>> {
    use sqlx::MySqlPool;
    use wyse_ontology::OntologyRepository;

    let pool = MySqlPool::connect(&std::env::var("DATABASE_URL")?).await?;
    let repository = SqlxOntologyRepository::new(pool);
    let revision = published_revision();
    let online = TagName::online();

    repository.insert_revision(revision.clone()).await?;
    repository.put_tag(&online, &revision.id).await?;

    assert_eq!(repository.get_tag(&online).await?, Some(revision.id));
    Ok(())
}

#[tokio::test]
#[ignore = "requires MySQL 8 started by the crate Makefile"]
async fn repository_rejects_a_revision_id_that_does_not_match_its_schema()
-> Result<(), Box<dyn std::error::Error>> {
    use sqlx::MySqlPool;

    let pool = MySqlPool::connect(&std::env::var("DATABASE_URL")?).await?;
    let repository = SqlxOntologyRepository::new(pool);
    let mut revision = published_revision();
    revision.id = RevisionId::try_from("0".repeat(64)).expect("syntactically valid revision id");

    assert!(repository.insert_revision(revision).await.is_err());
    Ok(())
}

#[tokio::test]
#[ignore = "requires MySQL 8 started by the crate Makefile"]
async fn repository_rejects_a_stored_revision_with_mismatched_schema_content()
-> Result<(), Box<dyn std::error::Error>> {
    use sqlx::MySqlPool;

    let pool = MySqlPool::connect(&std::env::var("DATABASE_URL")?).await?;
    let repository = SqlxOntologyRepository::new(pool.clone());
    let revision = published_revision();
    let corrupt_id = RevisionId::try_from("a".repeat(64)).expect("syntactically valid revision id");
    let schema_json = String::from_utf8(canonical_schema_bytes(&revision.schema)?)?;

    sqlx::query(
        "INSERT INTO ontology_revisions (revision_id, schema_json, schema_format_version) \
         VALUES (?, CAST(? AS JSON), ?)",
    )
    .bind(corrupt_id.as_str())
    .bind(schema_json)
    .bind(revision.schema.schema_version)
    .execute(&pool)
    .await?;

    assert!(repository.get_revision(&corrupt_id).await.is_err());
    Ok(())
}

#[tokio::test]
#[ignore = "requires MySQL 8 started by the crate Makefile"]
async fn repository_atomically_enforces_cardinality_and_excludes_replaced_link()
-> Result<(), Box<dyn std::error::Error>> {
    use sqlx::MySqlPool;

    let repository = Arc::new(SqlxOntologyRepository::new(
        MySqlPool::connect(&std::env::var("DATABASE_URL")?).await?,
    ));
    let object_type_id = ObjectTypeId::new();
    let source = ObjectId::new();
    let first_target = ObjectId::new();
    let second_target = ObjectId::new();
    for id in [source, first_target, second_target] {
        repository
            .create_object(NewObjectRecord {
                id,
                object_type_id,
                values: Map::new(),
            })
            .await?;
    }

    let link_type_id = LinkTypeId::new();
    let constraints = [LinkCardinalityConstraint {
        cardinality: Cardinality::ManyToOne,
    }];
    let barrier = Arc::new(Barrier::new(3));
    let first = {
        let barrier = barrier.clone();
        let repository = repository.clone();
        async move {
            barrier.wait().await;
            repository
                .create_link_with_cardinality(
                    NewLinkRecord {
                        id: LinkId::new(),
                        link_type_id,
                        source_object_id: source,
                        target_object_id: first_target,
                    },
                    &constraints,
                )
                .await
        }
    };
    let second = {
        let barrier = barrier.clone();
        let repository = repository.clone();
        async move {
            barrier.wait().await;
            repository
                .create_link_with_cardinality(
                    NewLinkRecord {
                        id: LinkId::new(),
                        link_type_id,
                        source_object_id: source,
                        target_object_id: second_target,
                    },
                    &constraints,
                )
                .await
        }
    };

    let both = async { tokio::join!(first, second) };
    let (_, (first, second)) = timeout(Duration::from_secs(10), async {
        tokio::join!(barrier.wait(), both)
    })
    .await
    .map_err(|_| "concurrent cardinality test timed out")?;
    let created = match (first, second) {
        (Ok(link), Err(OntologyError::CardinalityConflict { .. }))
        | (Err(OntologyError::CardinalityConflict { .. }), Ok(link)) => link,
        (left, right) => panic!("expected one cardinality conflict, got {left:?} and {right:?}"),
    };

    let replaced = repository
        .replace_link_with_cardinality(
            LinkRecord {
                id: created.id,
                link_type_id: created.link_type_id,
                source_object_id: created.source_object_id,
                target_object_id: created.target_object_id,
                version: created.version,
            },
            &constraints,
        )
        .await?;
    assert_eq!(replaced.version, created.version + 1);
    Ok(())
}

#[tokio::test]
#[ignore = "requires MySQL 8 started by the crate Makefile"]
async fn force_delete_serializes_with_link_creation() -> Result<(), Box<dyn std::error::Error>> {
    use sqlx::MySqlPool;

    let repository = Arc::new(SqlxOntologyRepository::new(
        MySqlPool::connect(&std::env::var("DATABASE_URL")?).await?,
    ));
    let object_type_id = ObjectTypeId::new();
    let source = ObjectId::new();
    let target = ObjectId::new();
    for id in [source, target] {
        repository
            .create_object(NewObjectRecord {
                id,
                object_type_id,
                values: Map::new(),
            })
            .await?;
    }

    let barrier = Arc::new(Barrier::new(3));
    let deleting = {
        let barrier = barrier.clone();
        let repository = repository.clone();
        async move {
            barrier.wait().await;
            repository.delete_object(source, 1, true).await
        }
    };
    let creating = {
        let barrier = barrier.clone();
        let repository = repository.clone();
        async move {
            barrier.wait().await;
            repository
                .create_link_with_cardinality(
                    NewLinkRecord {
                        id: LinkId::new(),
                        link_type_id: LinkTypeId::new(),
                        source_object_id: source,
                        target_object_id: target,
                    },
                    &[LinkCardinalityConstraint {
                        cardinality: Cardinality::ManyToMany,
                    }],
                )
                .await
        }
    };

    let both = async { tokio::join!(deleting, creating) };
    let (_, (deleting, creating)) = timeout(Duration::from_secs(10), async {
        tokio::join!(barrier.wait(), both)
    })
    .await
    .map_err(|_| "force delete race test timed out")?;

    assert!(deleting.is_ok());
    assert!(creating.is_ok() || matches!(creating, Err(OntologyError::ObjectMissing { .. })));
    assert!(repository.get_object(source).await?.is_none());
    assert!(
        repository
            .page_links(None, 100)
            .await?
            .items
            .iter()
            .all(|link| link.source_object_id != source && link.target_object_id != source)
    );
    Ok(())
}
