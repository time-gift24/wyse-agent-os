use std::sync::Arc;

use serde_json::Map;
use sqlx::{MySql, MySqlPool, mysql::MySqlPoolOptions, pool::PoolConnection};
use tokio::{
    sync::Barrier,
    time::{Duration, timeout},
};
use wyse_ontology::{
    Cardinality, LinkCardinalityConstraint, LinkId, LinkRecord, LinkType, LinkTypeId,
    NewLinkRecord, NewObjectRecord, ObjectId, ObjectType, ObjectTypeId, OntologyError,
    OntologyRepository, PublishedRevision, RevisionId, SchemaDocument, TagName,
    canonical_schema_bytes, revision_id,
};
use wyse_ontology_mysql::SqlxOntologyRepository;

const TEST_SUITE_LOCK: &str = "wyse_ontology_mysql_test_suite";

struct TestDatabase {
    database_url: String,
    _suite_lock: PoolConnection<MySql>,
    _setup_pool: MySqlPool,
}

impl TestDatabase {
    async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let database_url = std::env::var("DATABASE_URL")?;
        let setup_pool = MySqlPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await?;
        let mut suite_lock = setup_pool.acquire().await?;
        let acquired: Option<i64> = sqlx::query_scalar("SELECT GET_LOCK(?, 30)")
            .bind(TEST_SUITE_LOCK)
            .fetch_one(&mut *suite_lock)
            .await?;
        if acquired != Some(1) {
            return Err("could not acquire ontology MySQL test-suite lock".into());
        }
        for table in ["links", "objects", "ontology_tags", "ontology_revisions"] {
            sqlx::query(&format!("DELETE FROM {table}"))
                .execute(&mut *suite_lock)
                .await?;
        }
        Ok(Self {
            database_url,
            _suite_lock: suite_lock,
            _setup_pool: setup_pool,
        })
    }

    async fn pool(&self, max_connections: u32) -> Result<MySqlPool, sqlx::Error> {
        MySqlPoolOptions::new()
            .max_connections(max_connections)
            .connect(&self.database_url)
            .await
    }
}

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

async fn seed_revision(
    pool: &sqlx::MySqlPool,
    revision: &PublishedRevision,
) -> Result<(), sqlx::Error> {
    let schema_json = String::from_utf8(
        canonical_schema_bytes(&revision.schema)
            .expect("test fixture revisions have canonical schema bytes"),
    )
    .expect("canonical schema bytes are UTF-8");
    sqlx::query(
        "INSERT INTO ontology_revisions (revision_id, schema_json, schema_format_version) \
         VALUES (?, CAST(? AS JSON), ?)",
    )
    .bind(revision.id.as_str())
    .bind(schema_json)
    .bind(revision.schema.schema_version)
    .execute(pool)
    .await?;
    Ok(())
}

fn cardinality_revision(cardinality: Cardinality) -> PublishedRevision {
    let object_type_id = ObjectTypeId::from(uuid::Uuid::from_u128(1));
    let schema = SchemaDocument {
        schema_version: 1,
        object_types: vec![ObjectType {
            id: object_type_id,
            name: "person".to_owned(),
            description: String::new(),
            properties: Vec::new(),
        }],
        link_types: vec![LinkType::new(
            LinkTypeId::from(uuid::Uuid::from_u128(2)),
            "knows".to_owned(),
            object_type_id,
            object_type_id,
            cardinality,
        )],
    };
    let id = revision_id(&schema).expect("valid schema has a revision id");
    PublishedRevision { id, schema }
}

async fn seed_permissive_links(
    repository: &SqlxOntologyRepository,
    pool: &MySqlPool,
    pairs: &[(ObjectId, ObjectId)],
) -> Result<PublishedRevision, Box<dyn std::error::Error>> {
    let revision = cardinality_revision(Cardinality::ManyToMany);
    seed_revision(pool, &revision).await?;
    repository.move_online_tag(&revision.id).await?;
    let object_type_id = revision.schema.object_types[0].id;
    for id in [10_u128, 11, 12].map(|value| ObjectId::from(uuid::Uuid::from_u128(value))) {
        repository
            .create_object(
                NewObjectRecord {
                    id,
                    object_type_id,
                    values: Map::new(),
                },
                &revision.id,
            )
            .await?;
    }
    for &(source_object_id, target_object_id) in pairs {
        repository
            .create_link_with_cardinality(
                NewLinkRecord {
                    id: LinkId::new(),
                    link_type_id: revision.schema.link_types[0].id,
                    source_object_id,
                    target_object_id,
                },
                &[LinkCardinalityConstraint {
                    cardinality: Cardinality::ManyToMany,
                }],
                &revision.id,
            )
            .await?;
    }
    Ok(revision)
}

#[tokio::test]
#[ignore = "requires MySQL 8 started by the crate Makefile"]
async fn repository_persists_revision_and_online_tag() -> Result<(), Box<dyn std::error::Error>> {
    use wyse_ontology::OntologyRepository;

    let database = TestDatabase::new().await?;
    let pool = database.pool(5).await?;
    let repository = SqlxOntologyRepository::new(pool.clone());
    let revision = published_revision();
    let online = TagName::online();

    seed_revision(&pool, &revision).await?;
    repository.put_tag(&online, &revision.id).await?;

    assert_eq!(repository.get_tag(&online).await?, Some(revision.id));
    Ok(())
}

#[tokio::test]
#[ignore = "requires MySQL 8 started by the crate Makefile"]
async fn repository_rejects_a_revision_id_that_does_not_match_its_schema()
-> Result<(), Box<dyn std::error::Error>> {
    let database = TestDatabase::new().await?;
    let pool = database.pool(5).await?;
    let repository = SqlxOntologyRepository::new(pool);
    let mut revision = published_revision();
    revision.id = RevisionId::try_from("0".repeat(64)).expect("syntactically valid revision id");

    assert!(repository.publish_revision(revision).await.is_err());
    Ok(())
}

#[tokio::test]
#[ignore = "requires MySQL 8 started by the crate Makefile"]
async fn repository_rejects_a_stored_revision_with_mismatched_schema_content()
-> Result<(), Box<dyn std::error::Error>> {
    let database = TestDatabase::new().await?;
    let pool = database.pool(5).await?;
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
async fn moving_online_rejects_a_revision_incompatible_with_existing_instances()
-> Result<(), Box<dyn std::error::Error>> {
    let database = TestDatabase::new().await?;
    let pool = database.pool(5).await?;
    let repository = SqlxOntologyRepository::new(pool.clone());
    let object_type_id = ObjectTypeId::new();
    let compatible_schema = SchemaDocument {
        schema_version: 1,
        object_types: vec![ObjectType {
            id: object_type_id,
            name: "person".to_owned(),
            description: String::new(),
            properties: Vec::new(),
        }],
        link_types: Vec::new(),
    };
    let compatible = PublishedRevision {
        id: revision_id(&compatible_schema)?,
        schema: compatible_schema,
    };
    seed_revision(&pool, &compatible).await?;
    repository.move_online_tag(&compatible.id).await?;
    repository
        .create_object(
            NewObjectRecord {
                id: ObjectId::new(),
                object_type_id,
                values: Map::new(),
            },
            &compatible.id,
        )
        .await?;

    let incompatible_schema = SchemaDocument {
        schema_version: 1,
        object_types: Vec::new(),
        link_types: Vec::new(),
    };
    let incompatible = PublishedRevision {
        id: revision_id(&incompatible_schema)?,
        schema: incompatible_schema,
    };
    seed_revision(&pool, &incompatible).await?;

    assert!(matches!(
        repository.move_online_tag(&incompatible.id).await,
        Err(OntologyError::PublishInvalid { .. })
    ));
    assert_eq!(
        repository.get_tag(&TagName::online()).await?,
        Some(compatible.id)
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires MySQL 8 started by the crate Makefile"]
async fn repository_atomically_enforces_cardinality_and_excludes_replaced_link()
-> Result<(), Box<dyn std::error::Error>> {
    let database = TestDatabase::new().await?;
    let pool = database.pool(5).await?;
    let repository = Arc::new(SqlxOntologyRepository::new(pool.clone()));
    let online = published_revision();
    seed_revision(&pool, &online).await?;
    repository.put_tag(&TagName::online(), &online.id).await?;
    let object_type_id = ObjectTypeId::new();
    let source = ObjectId::new();
    let first_target = ObjectId::new();
    let second_target = ObjectId::new();
    for id in [source, first_target, second_target] {
        repository
            .create_object(
                NewObjectRecord {
                    id,
                    object_type_id,
                    values: Map::new(),
                },
                &online.id,
            )
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
        let online_id = online.id.clone();
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
                    &online_id,
                )
                .await
        }
    };
    let second = {
        let barrier = barrier.clone();
        let repository = repository.clone();
        let online_id = online.id.clone();
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
                    &online_id,
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
            &online.id,
        )
        .await?;
    assert_eq!(replaced.version, created.version + 1);
    Ok(())
}

#[tokio::test]
#[ignore = "requires MySQL 8 started by the crate Makefile"]
async fn force_delete_serializes_with_link_creation() -> Result<(), Box<dyn std::error::Error>> {
    let database = TestDatabase::new().await?;
    let pool = database.pool(5).await?;
    let repository = Arc::new(SqlxOntologyRepository::new(pool.clone()));
    let online = published_revision();
    seed_revision(&pool, &online).await?;
    repository.put_tag(&TagName::online(), &online.id).await?;
    let object_type_id = ObjectTypeId::new();
    let source = ObjectId::new();
    let target = ObjectId::new();
    for id in [source, target] {
        repository
            .create_object(
                NewObjectRecord {
                    id,
                    object_type_id,
                    values: Map::new(),
                },
                &online.id,
            )
            .await?;
    }

    let barrier = Arc::new(Barrier::new(3));
    let deleting = {
        let barrier = barrier.clone();
        let repository = repository.clone();
        let online_id = online.id.clone();
        async move {
            barrier.wait().await;
            repository.delete_object(source, 1, true, &online_id).await
        }
    };
    let creating = {
        let barrier = barrier.clone();
        let repository = repository.clone();
        let online_id = online.id.clone();
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
                    &online_id,
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

#[tokio::test]
#[ignore = "requires MySQL 8 started by the crate Makefile"]
async fn publishing_rejects_existing_links_when_cardinality_becomes_stricter()
-> Result<(), Box<dyn std::error::Error>> {
    let database = TestDatabase::new().await?;
    let pool = database.pool(5).await?;
    let repository = SqlxOntologyRepository::new(pool.clone());
    let source = ObjectId::from(uuid::Uuid::from_u128(10));
    let first_target = ObjectId::from(uuid::Uuid::from_u128(11));
    let second_target = ObjectId::from(uuid::Uuid::from_u128(12));
    seed_permissive_links(
        &repository,
        &pool,
        &[(source, first_target), (source, second_target)],
    )
    .await?;
    let stricter = cardinality_revision(Cardinality::ManyToOne);

    assert!(matches!(
        repository.publish_revision(stricter.clone()).await,
        Err(OntologyError::PublishInvalid { .. })
    ));
    assert!(repository.get_revision(&stricter.id).await?.is_none());
    Ok(())
}

#[tokio::test]
#[ignore = "requires MySQL 8 started by the crate Makefile"]
async fn moving_online_rejects_existing_links_when_cardinality_becomes_stricter()
-> Result<(), Box<dyn std::error::Error>> {
    let database = TestDatabase::new().await?;
    let pool = database.pool(5).await?;
    let repository = SqlxOntologyRepository::new(pool.clone());
    let first_source = ObjectId::from(uuid::Uuid::from_u128(10));
    let second_source = ObjectId::from(uuid::Uuid::from_u128(11));
    let target = ObjectId::from(uuid::Uuid::from_u128(12));
    let permissive = seed_permissive_links(
        &repository,
        &pool,
        &[(first_source, target), (second_source, target)],
    )
    .await?;
    let stricter = cardinality_revision(Cardinality::OneToMany);
    seed_revision(&pool, &stricter).await?;

    assert!(matches!(
        repository.move_online_tag(&stricter.id).await,
        Err(OntologyError::PublishInvalid { .. })
    ));
    assert_eq!(
        repository.get_tag(&TagName::online()).await?,
        Some(permissive.id)
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires MySQL 8 started by the crate Makefile"]
async fn replace_object_version_conflict_completes_with_one_connection()
-> Result<(), Box<dyn std::error::Error>> {
    let database = TestDatabase::new().await?;
    let pool = database.pool(1).await?;
    let repository = SqlxOntologyRepository::new(pool.clone());
    let revision = published_revision();
    seed_revision(&pool, &revision).await?;
    repository.move_online_tag(&revision.id).await?;
    let object = repository
        .create_object(
            NewObjectRecord {
                id: ObjectId::new(),
                object_type_id: revision.schema.object_types[0].id,
                values: Map::new(),
            },
            &revision.id,
        )
        .await?;

    let result = timeout(
        Duration::from_secs(2),
        repository.replace_object(
            wyse_ontology::ObjectRecord {
                version: 0,
                ..object
            },
            &revision.id,
        ),
    )
    .await
    .map_err(|_| "replace-object conflict deadlocked with one connection")?;

    assert!(matches!(
        result,
        Err(OntologyError::ObjectVersionConflict { .. })
    ));
    Ok(())
}

#[tokio::test]
#[ignore = "requires MySQL 8 started by the crate Makefile"]
async fn delete_object_version_conflict_completes_with_one_connection()
-> Result<(), Box<dyn std::error::Error>> {
    let database = TestDatabase::new().await?;
    let pool = database.pool(1).await?;
    let repository = SqlxOntologyRepository::new(pool.clone());
    let revision = published_revision();
    seed_revision(&pool, &revision).await?;
    repository.move_online_tag(&revision.id).await?;
    let object = repository
        .create_object(
            NewObjectRecord {
                id: ObjectId::new(),
                object_type_id: revision.schema.object_types[0].id,
                values: Map::new(),
            },
            &revision.id,
        )
        .await?;

    let result = timeout(
        Duration::from_secs(2),
        repository.delete_object(object.id, 0, false, &revision.id),
    )
    .await
    .map_err(|_| "delete-object conflict deadlocked with one connection")?;

    assert!(matches!(
        result,
        Err(OntologyError::ObjectVersionConflict { .. })
    ));
    Ok(())
}

#[tokio::test]
#[ignore = "requires MySQL 8 started by the crate Makefile"]
async fn delete_link_version_conflict_completes_with_one_connection()
-> Result<(), Box<dyn std::error::Error>> {
    let database = TestDatabase::new().await?;
    let pool = database.pool(1).await?;
    let repository = SqlxOntologyRepository::new(pool.clone());
    let revision = cardinality_revision(Cardinality::ManyToMany);
    seed_revision(&pool, &revision).await?;
    repository.move_online_tag(&revision.id).await?;
    let source = ObjectId::from(uuid::Uuid::from_u128(10));
    let target = ObjectId::from(uuid::Uuid::from_u128(11));
    for id in [source, target] {
        repository
            .create_object(
                NewObjectRecord {
                    id,
                    object_type_id: revision.schema.object_types[0].id,
                    values: Map::new(),
                },
                &revision.id,
            )
            .await?;
    }
    let link = repository
        .create_link_with_cardinality(
            NewLinkRecord {
                id: LinkId::new(),
                link_type_id: revision.schema.link_types[0].id,
                source_object_id: source,
                target_object_id: target,
            },
            &[LinkCardinalityConstraint {
                cardinality: Cardinality::ManyToMany,
            }],
            &revision.id,
        )
        .await?;

    let result = timeout(
        Duration::from_secs(2),
        repository.delete_link(link.id, 0, &revision.id),
    )
    .await
    .map_err(|_| "delete-link conflict deadlocked with one connection")?;

    assert!(matches!(
        result,
        Err(OntologyError::LinkVersionConflict { .. })
    ));
    Ok(())
}
