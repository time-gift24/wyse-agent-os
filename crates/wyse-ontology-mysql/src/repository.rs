//! MySQL implementation of the ontology repository contract.

use std::str::FromStr;

use async_trait::async_trait;
use serde_json::{Map, Value};
use sqlx::{Acquire, MySql, MySqlPool, Row, Transaction, pool::PoolConnection};
use uuid::Uuid;
use wyse_ontology::{
    Cardinality, LinkCardinalityConstraint, LinkId, LinkRecord, NewLinkRecord, NewObjectRecord,
    ObjectId, ObjectRecord, ObjectTypeId, OntologyError, OntologyRepository, Page,
    PublishedRevision, RevisionId, TagName, canonical_schema_bytes, validate_published_revision,
    validate_schema_instances,
};

use crate::error::MySqlOntologyRepositoryError;

/// MySQL 8 repository for immutable revisions and shared ontology instances.
#[derive(Clone)]
pub struct SqlxOntologyRepository {
    pool: MySqlPool,
}

const INSTANCE_WRITE_LOCK: &str = "wyse_ontology_instance_write";
const INSTANCE_WRITE_LOCK_TIMEOUT_SECONDS: i32 = 10;

struct InstanceWriteLock {
    connection: Option<PoolConnection<MySql>>,
    released: bool,
}

impl InstanceWriteLock {
    async fn acquire(pool: &MySqlPool) -> Result<Self, OntologyError> {
        let mut connection = pool.acquire().await.map_err(sqlx_error)?;
        let acquired: Option<i64> = sqlx::query_scalar("SELECT GET_LOCK(?, ?)")
            .bind(INSTANCE_WRITE_LOCK)
            .bind(INSTANCE_WRITE_LOCK_TIMEOUT_SECONDS)
            .fetch_one(&mut *connection)
            .await
            .map_err(sqlx_error)?;
        if acquired == Some(1) {
            Ok(Self {
                connection: Some(connection),
                released: false,
            })
        } else {
            Err(repository_error(
                MySqlOntologyRepositoryError::InstanceLockUnavailable,
            ))
        }
    }

    fn connection_mut(&mut self) -> &mut PoolConnection<MySql> {
        self.connection
            .as_mut()
            .expect("active instance-write lock retains its connection")
    }

    async fn release(mut self) -> Result<(), OntologyError> {
        let released: Result<Option<i64>, sqlx::Error> =
            sqlx::query_scalar("SELECT RELEASE_LOCK(?)")
                .bind(INSTANCE_WRITE_LOCK)
                .fetch_one(&mut **self.connection_mut())
                .await;
        if matches!(&released, Ok(Some(1))) {
            self.released = true;
            return Ok(());
        }

        if let Some(connection) = &mut self.connection {
            connection.close_on_drop();
        }
        match released {
            Err(source) => Err(sqlx_error(source)),
            Ok(_) => Err(repository_error(
                MySqlOntologyRepositoryError::InstanceLockReleaseFailed,
            )),
        }
    }
}

impl Drop for InstanceWriteLock {
    fn drop(&mut self) {
        if !self.released
            && let Some(connection) = &mut self.connection
        {
            connection.close_on_drop();
        }
    }
}

async fn finish_instance_write<T>(
    lock: InstanceWriteLock,
    operation: Result<T, OntologyError>,
) -> Result<T, OntologyError> {
    let release = lock.release().await;
    match (operation, release) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

impl SqlxOntologyRepository {
    /// Creates a repository using the supplied MySQL connection pool.
    #[must_use]
    pub fn new(pool: MySqlPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl OntologyRepository for SqlxOntologyRepository {
    async fn publish_revision(&self, revision: PublishedRevision) -> Result<(), OntologyError> {
        validate_published_revision(&revision)?;
        let mut lock = InstanceWriteLock::acquire(&self.pool).await?;
        let operation = async {
            let mut transaction = lock.connection_mut().begin().await.map_err(sqlx_error)?;
            let objects = object_records_in_transaction(&mut transaction).await?;
            let links = link_records_in_transaction(&mut transaction).await?;
            validate_schema_instances(&revision.schema, &objects, &links)?;
            insert_revision_transaction(&mut transaction, &revision).await?;
            transaction.commit().await.map_err(sqlx_error)
        }
        .await;
        finish_instance_write(lock, operation).await
    }

    async fn get_revision(
        &self,
        id: &RevisionId,
    ) -> Result<Option<PublishedRevision>, OntologyError> {
        let row = sqlx::query(
            "SELECT revision_id, CAST(schema_json AS CHAR) AS schema_json \
             FROM ontology_revisions WHERE revision_id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_error)?;
        row.map(revision_from_row).transpose()
    }

    async fn list_revisions(&self) -> Result<Vec<PublishedRevision>, OntologyError> {
        sqlx::query(
            "SELECT revision_id, CAST(schema_json AS CHAR) AS schema_json \
             FROM ontology_revisions ORDER BY created_at, revision_id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_error)?
        .into_iter()
        .map(revision_from_row)
        .collect()
    }

    async fn put_tag(&self, name: &TagName, revision_id: &RevisionId) -> Result<(), OntologyError> {
        sqlx::query(
            "INSERT INTO ontology_tags (name, revision_id) VALUES (?, ?) \
             ON DUPLICATE KEY UPDATE revision_id = VALUES(revision_id), updated_at = UTC_TIMESTAMP(6)",
        )
        .bind(name.as_str())
        .bind(revision_id.as_str())
        .execute(&self.pool)
        .await
        .map_err(sqlx_error)?;
        Ok(())
    }

    async fn move_online_tag(&self, revision_id: &RevisionId) -> Result<(), OntologyError> {
        let mut lock = InstanceWriteLock::acquire(&self.pool).await?;
        let operation = async {
            let mut transaction = lock.connection_mut().begin().await.map_err(sqlx_error)?;
            let revision = revision_in_transaction(&mut transaction, revision_id)
                .await?
                .ok_or_else(|| OntologyError::RevisionMissing {
                    id: revision_id.clone(),
                })?;
            let objects = object_records_in_transaction(&mut transaction).await?;
            let links = link_records_in_transaction(&mut transaction).await?;
            validate_schema_instances(&revision.schema, &objects, &links)?;
            put_tag_transaction(&mut transaction, &TagName::online(), revision_id).await?;
            transaction.commit().await.map_err(sqlx_error)
        }
        .await;
        finish_instance_write(lock, operation).await
    }

    async fn get_tag(&self, name: &TagName) -> Result<Option<RevisionId>, OntologyError> {
        let row = sqlx::query("SELECT revision_id FROM ontology_tags WHERE name = ?")
            .bind(name.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(sqlx_error)?;
        row.map(|row| revision_id_from_str(row.get::<String, _>("revision_id")))
            .transpose()
    }

    async fn delete_tag(&self, name: &TagName) -> Result<(), OntologyError> {
        sqlx::query("DELETE FROM ontology_tags WHERE name = ?")
            .bind(name.as_str())
            .execute(&self.pool)
            .await
            .map_err(sqlx_error)?;
        Ok(())
    }

    async fn create_object(
        &self,
        object: NewObjectRecord,
        online_revision_id: &RevisionId,
    ) -> Result<ObjectRecord, OntologyError> {
        let values = serde_json::to_string(&object.values).map_err(encode_values_error)?;
        let mut lock = InstanceWriteLock::acquire(&self.pool).await?;
        let operation = async {
            let mut transaction = lock.connection_mut().begin().await.map_err(sqlx_error)?;
            ensure_online_revision(&mut transaction, online_revision_id).await?;
            sqlx::query(
                "INSERT INTO objects (id, object_type_id, values_json, version) VALUES (?, ?, CAST(? AS JSON), 1)",
            )
            .bind(object.id.to_string())
            .bind(object.object_type_id.to_string())
            .bind(values)
            .execute(&mut *transaction)
            .await
            .map_err(sqlx_error)?;
            transaction.commit().await.map_err(sqlx_error)?;
            Ok(ObjectRecord {
                id: object.id,
                object_type_id: object.object_type_id,
                values: object.values,
                version: 1,
            })
        }
        .await;
        finish_instance_write(lock, operation).await
    }

    async fn get_object(&self, id: ObjectId) -> Result<Option<ObjectRecord>, OntologyError> {
        sqlx::query(
            "SELECT id, object_type_id, CAST(values_json AS CHAR) AS values_json, version \
             FROM objects WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_error)?
        .map(object_from_row)
        .transpose()
    }

    async fn page_objects(
        &self,
        type_id: ObjectTypeId,
        after: Option<ObjectId>,
        limit: u32,
    ) -> Result<Page<ObjectRecord>, OntologyError> {
        if limit == 0 {
            return Ok(Page {
                items: Vec::new(),
                next_after: None,
            });
        }
        let rows = match after {
            Some(after) => {
                sqlx::query(
                    "SELECT id, object_type_id, CAST(values_json AS CHAR) AS values_json, version FROM objects \
                 WHERE object_type_id = ? AND id > ? ORDER BY id LIMIT ?",
                )
                .bind(type_id.to_string())
                .bind(after.to_string())
                .bind(i64::from(limit) + 1)
                .fetch_all(&self.pool)
                .await
            }
            None => {
                sqlx::query(
                    "SELECT id, object_type_id, CAST(values_json AS CHAR) AS values_json, version FROM objects \
                 WHERE object_type_id = ? ORDER BY id LIMIT ?",
                )
                .bind(type_id.to_string())
                .bind(i64::from(limit) + 1)
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(sqlx_error)?;
        page(rows, limit, object_from_row, |object| object.id.as_uuid())
    }

    async fn replace_object(
        &self,
        object: ObjectRecord,
        online_revision_id: &RevisionId,
    ) -> Result<ObjectRecord, OntologyError> {
        let values = serde_json::to_string(&object.values).map_err(encode_values_error)?;
        let mut lock = InstanceWriteLock::acquire(&self.pool).await?;
        let operation = async {
            let mut transaction = lock.connection_mut().begin().await.map_err(sqlx_error)?;
            ensure_online_revision(&mut transaction, online_revision_id).await?;
            let result = sqlx::query(
                "UPDATE objects SET values_json = CAST(? AS JSON), version = version + 1, updated_at = UTC_TIMESTAMP(6) \
                 WHERE id = ? AND version = ?",
            )
            .bind(values)
            .bind(object.id.to_string())
            .bind(object.version)
            .execute(&mut *transaction)
            .await
            .map_err(sqlx_error)?;
            if result.rows_affected() == 0 {
                let exists = object_exists(&mut transaction, object.id).await?;
                return Err(object_write_failure_from_exists(object.id, exists));
            }
            transaction.commit().await.map_err(sqlx_error)?;
            Ok(ObjectRecord {
                version: object.version + 1,
                ..object
            })
        }
        .await;
        finish_instance_write(lock, operation).await
    }

    async fn delete_object(
        &self,
        id: ObjectId,
        version: u64,
        force: bool,
        online_revision_id: &RevisionId,
    ) -> Result<(), OntologyError> {
        let mut lock = InstanceWriteLock::acquire(&self.pool).await?;
        let operation = async {
            if force {
                return delete_object_force(lock.connection_mut(), id, version, online_revision_id)
                    .await;
            }
            let mut transaction = lock.connection_mut().begin().await.map_err(sqlx_error)?;
            ensure_online_revision(&mut transaction, online_revision_id).await?;
            let result = sqlx::query("DELETE FROM objects WHERE id = ? AND version = ?")
                .bind(id.to_string())
                .bind(version)
                .execute(&mut *transaction)
                .await
                .map_err(|error| map_object_delete_error(id, error))?;
            if result.rows_affected() == 0 {
                let exists = object_exists(&mut transaction, id).await?;
                return Err(object_write_failure_from_exists(id, exists));
            }
            transaction.commit().await.map_err(sqlx_error)
        }
        .await;
        finish_instance_write(lock, operation).await
    }

    async fn create_link_with_cardinality(
        &self,
        link: NewLinkRecord,
        constraints: &[LinkCardinalityConstraint],
        online_revision_id: &RevisionId,
    ) -> Result<LinkRecord, OntologyError> {
        create_link_with_cardinality(&self.pool, link, constraints, online_revision_id).await
    }

    async fn get_link(&self, id: LinkId) -> Result<Option<LinkRecord>, OntologyError> {
        sqlx::query("SELECT id, link_type_id, source_object_id, target_object_id, version FROM links WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(sqlx_error)?
            .map(link_from_row)
            .transpose()
    }

    async fn page_links(
        &self,
        after: Option<LinkId>,
        limit: u32,
    ) -> Result<Page<LinkRecord>, OntologyError> {
        if limit == 0 {
            return Ok(Page {
                items: Vec::new(),
                next_after: None,
            });
        }
        let rows = match after {
            Some(after) => sqlx::query(
                "SELECT id, link_type_id, source_object_id, target_object_id, version FROM links \
                 WHERE id > ? ORDER BY id LIMIT ?",
            )
            .bind(after.to_string())
            .bind(i64::from(limit) + 1)
            .fetch_all(&self.pool)
            .await,
            None => sqlx::query(
                "SELECT id, link_type_id, source_object_id, target_object_id, version FROM links \
                 ORDER BY id LIMIT ?",
            )
            .bind(i64::from(limit) + 1)
            .fetch_all(&self.pool)
            .await,
        }
        .map_err(sqlx_error)?;
        page(rows, limit, link_from_row, |link| link.id.as_uuid())
    }

    async fn replace_link_with_cardinality(
        &self,
        link: LinkRecord,
        constraints: &[LinkCardinalityConstraint],
        online_revision_id: &RevisionId,
    ) -> Result<LinkRecord, OntologyError> {
        replace_link_with_cardinality(&self.pool, link, constraints, online_revision_id).await
    }

    async fn delete_link(
        &self,
        id: LinkId,
        version: u64,
        online_revision_id: &RevisionId,
    ) -> Result<(), OntologyError> {
        let mut lock = InstanceWriteLock::acquire(&self.pool).await?;
        let operation = async {
            let mut transaction = lock.connection_mut().begin().await.map_err(sqlx_error)?;
            ensure_online_revision(&mut transaction, online_revision_id).await?;
            let result = sqlx::query("DELETE FROM links WHERE id = ? AND version = ?")
                .bind(id.to_string())
                .bind(version)
                .execute(&mut *transaction)
                .await
                .map_err(sqlx_error)?;
            if result.rows_affected() == 0 {
                let exists = link_exists(&mut transaction, id).await?;
                return Err(link_write_failure_from_exists(id, exists));
            }
            transaction.commit().await.map_err(sqlx_error)
        }
        .await;
        finish_instance_write(lock, operation).await
    }
}

async fn object_records_in_transaction(
    transaction: &mut Transaction<'_, MySql>,
) -> Result<Vec<ObjectRecord>, OntologyError> {
    sqlx::query(
        "SELECT id, object_type_id, CAST(values_json AS CHAR) AS values_json, version \
         FROM objects ORDER BY id",
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(sqlx_error)?
    .into_iter()
    .map(object_from_row)
    .collect()
}

async fn link_records_in_transaction(
    transaction: &mut Transaction<'_, MySql>,
) -> Result<Vec<LinkRecord>, OntologyError> {
    sqlx::query(
        "SELECT id, link_type_id, source_object_id, target_object_id, version FROM links ORDER BY id",
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(sqlx_error)?
    .into_iter()
    .map(link_from_row)
    .collect()
}

async fn insert_revision_transaction(
    transaction: &mut Transaction<'_, MySql>,
    revision: &PublishedRevision,
) -> Result<(), OntologyError> {
    let schema_json =
        String::from_utf8(canonical_schema_bytes(&revision.schema)?).map_err(|_| {
            repository_error(MySqlOntologyRepositoryError::InvalidPersisted {
                kind: "canonical schema UTF-8",
            })
        })?;
    sqlx::query(
        "INSERT INTO ontology_revisions (revision_id, schema_json, schema_format_version) \
         VALUES (?, CAST(? AS JSON), ?) \
         ON DUPLICATE KEY UPDATE revision_id = revision_id",
    )
    .bind(revision.id.as_str())
    .bind(schema_json)
    .bind(revision.schema.schema_version)
    .execute(&mut **transaction)
    .await
    .map_err(sqlx_error)?;
    Ok(())
}

async fn revision_in_transaction(
    transaction: &mut Transaction<'_, MySql>,
    id: &RevisionId,
) -> Result<Option<PublishedRevision>, OntologyError> {
    sqlx::query(
        "SELECT revision_id, CAST(schema_json AS CHAR) AS schema_json \
         FROM ontology_revisions WHERE revision_id = ?",
    )
    .bind(id.as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(sqlx_error)?
    .map(revision_from_row)
    .transpose()
}

async fn put_tag_transaction(
    transaction: &mut Transaction<'_, MySql>,
    name: &TagName,
    revision_id: &RevisionId,
) -> Result<(), OntologyError> {
    sqlx::query(
        "INSERT INTO ontology_tags (name, revision_id) VALUES (?, ?) \
         ON DUPLICATE KEY UPDATE revision_id = VALUES(revision_id), updated_at = UTC_TIMESTAMP(6)",
    )
    .bind(name.as_str())
    .bind(revision_id.as_str())
    .execute(&mut **transaction)
    .await
    .map_err(sqlx_error)?;
    Ok(())
}

async fn ensure_online_revision(
    transaction: &mut Transaction<'_, MySql>,
    expected: &RevisionId,
) -> Result<(), OntologyError> {
    let online = TagName::online();
    let actual = sqlx::query("SELECT revision_id FROM ontology_tags WHERE name = ? FOR UPDATE")
        .bind(online.as_str())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(sqlx_error)?
        .map(|row| revision_id_from_str(row.get::<String, _>("revision_id")))
        .transpose()?
        .ok_or(OntologyError::TagMissing { name: online })?;
    if actual == *expected {
        Ok(())
    } else {
        Err(OntologyError::OnlineRevisionChanged)
    }
}

async fn create_link_with_cardinality(
    pool: &MySqlPool,
    link: NewLinkRecord,
    constraints: &[LinkCardinalityConstraint],
    online_revision_id: &RevisionId,
) -> Result<LinkRecord, OntologyError> {
    let mut lock = InstanceWriteLock::acquire(pool).await?;
    let operation = async {
        let mut transaction = lock.connection_mut().begin().await.map_err(sqlx_error)?;
        ensure_online_revision(&mut transaction, online_revision_id).await?;
        lock_objects(
            &mut transaction,
            [link.source_object_id, link.target_object_id].into_iter(),
        )
        .await?;
        ensure_cardinality(&mut transaction, &link_record(&link), constraints, None).await?;
        sqlx::query(
            "INSERT INTO links (id, link_type_id, source_object_id, target_object_id, version) \
             VALUES (?, ?, ?, ?, 1)",
        )
        .bind(link.id.to_string())
        .bind(link.link_type_id.to_string())
        .bind(link.source_object_id.to_string())
        .bind(link.target_object_id.to_string())
        .execute(&mut *transaction)
        .await
        .map_err(sqlx_error)?;
        transaction.commit().await.map_err(sqlx_error)?;
        Ok(link_record(&link))
    }
    .await;
    finish_instance_write(lock, operation).await
}

async fn replace_link_with_cardinality(
    pool: &MySqlPool,
    link: LinkRecord,
    constraints: &[LinkCardinalityConstraint],
    online_revision_id: &RevisionId,
) -> Result<LinkRecord, OntologyError> {
    let mut lock = InstanceWriteLock::acquire(pool).await?;
    let operation = async {
        let mut transaction = lock.connection_mut().begin().await.map_err(sqlx_error)?;
        ensure_online_revision(&mut transaction, online_revision_id).await?;
        let current = sqlx::query(
        "SELECT id, link_type_id, source_object_id, target_object_id, version FROM links WHERE id = ? FOR UPDATE",
        )
        .bind(link.id.to_string())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(sqlx_error)?
        .map(link_from_row)
        .transpose()?;
        let Some(current) = current else {
            return Err(OntologyError::LinkMissing { id: link.id });
        };
        if current.version != link.version {
            return Err(OntologyError::LinkVersionConflict { id: link.id });
        }
        lock_objects(
        &mut transaction,
        [
            current.source_object_id,
            current.target_object_id,
            link.source_object_id,
            link.target_object_id,
        ]
        .into_iter(),
        )
        .await?;
        ensure_cardinality(&mut transaction, &link, constraints, Some(link.id)).await?;
        sqlx::query(
        "UPDATE links SET source_object_id = ?, target_object_id = ?, version = version + 1, updated_at = UTC_TIMESTAMP(6) \
         WHERE id = ? AND version = ?",
        )
        .bind(link.source_object_id.to_string())
        .bind(link.target_object_id.to_string())
        .bind(link.id.to_string())
        .bind(link.version)
        .execute(&mut *transaction)
        .await
        .map_err(sqlx_error)?;
        let updated = LinkRecord {
            version: link.version + 1,
            ..link
        };
        transaction.commit().await.map_err(sqlx_error)?;
        Ok(updated)
    }
    .await;
    finish_instance_write(lock, operation).await
}

fn link_record(link: &NewLinkRecord) -> LinkRecord {
    LinkRecord {
        id: link.id,
        link_type_id: link.link_type_id,
        source_object_id: link.source_object_id,
        target_object_id: link.target_object_id,
        version: 1,
    }
}

async fn lock_objects(
    transaction: &mut Transaction<'_, MySql>,
    ids: impl Iterator<Item = ObjectId>,
) -> Result<(), OntologyError> {
    let mut ids = ids.collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    for id in ids {
        let locked = sqlx::query("SELECT id FROM objects WHERE id = ? FOR UPDATE")
            .bind(id.to_string())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(sqlx_error)?;
        if locked.is_none() {
            return Err(OntologyError::ObjectMissing { id });
        }
    }
    Ok(())
}

async fn ensure_cardinality(
    transaction: &mut Transaction<'_, MySql>,
    candidate: &LinkRecord,
    constraints: &[LinkCardinalityConstraint],
    excluding: Option<LinkId>,
) -> Result<(), OntologyError> {
    let rows = match excluding {
        Some(excluding) => sqlx::query(
            "SELECT id, link_type_id, source_object_id, target_object_id, version FROM links \
             WHERE link_type_id = ? AND (source_object_id = ? OR target_object_id = ?) AND id <> ? FOR UPDATE",
        )
        .bind(candidate.link_type_id.to_string())
        .bind(candidate.source_object_id.to_string())
        .bind(candidate.target_object_id.to_string())
        .bind(excluding.to_string())
        .fetch_all(&mut **transaction)
        .await,
        None => sqlx::query(
            "SELECT id, link_type_id, source_object_id, target_object_id, version FROM links \
             WHERE link_type_id = ? AND (source_object_id = ? OR target_object_id = ?) FOR UPDATE",
        )
        .bind(candidate.link_type_id.to_string())
        .bind(candidate.source_object_id.to_string())
        .bind(candidate.target_object_id.to_string())
        .fetch_all(&mut **transaction)
        .await,
    }
    .map_err(sqlx_error)?;
    let links = rows
        .into_iter()
        .map(link_from_row)
        .collect::<Result<Vec<_>, _>>()?;
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

async fn delete_object_force(
    connection: &mut PoolConnection<MySql>,
    id: ObjectId,
    version: u64,
    online_revision_id: &RevisionId,
) -> Result<(), OntologyError> {
    let mut transaction = connection.begin().await.map_err(sqlx_error)?;
    ensure_online_revision(&mut transaction, online_revision_id).await?;
    lock_objects(&mut transaction, std::iter::once(id)).await?;
    sqlx::query("DELETE FROM links WHERE source_object_id = ? OR target_object_id = ?")
        .bind(id.to_string())
        .bind(id.to_string())
        .execute(&mut *transaction)
        .await
        .map_err(sqlx_error)?;
    let result = sqlx::query("DELETE FROM objects WHERE id = ? AND version = ?")
        .bind(id.to_string())
        .bind(version)
        .execute(&mut *transaction)
        .await
        .map_err(sqlx_error)?;
    if result.rows_affected() == 0 {
        let exists = object_exists(&mut transaction, id).await?;
        transaction.rollback().await.map_err(sqlx_error)?;
        return Err(object_write_failure_from_exists(id, exists));
    }
    transaction.commit().await.map_err(sqlx_error)
}

async fn object_exists(
    transaction: &mut Transaction<'_, MySql>,
    id: ObjectId,
) -> Result<bool, OntologyError> {
    sqlx::query("SELECT 1 FROM objects WHERE id = ?")
        .bind(id.to_string())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(sqlx_error)
        .map(|row| row.is_some())
}

async fn link_exists(
    transaction: &mut Transaction<'_, MySql>,
    id: LinkId,
) -> Result<bool, OntologyError> {
    Ok(sqlx::query("SELECT 1 FROM links WHERE id = ?")
        .bind(id.to_string())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(sqlx_error)?
        .is_some())
}

fn link_write_failure_from_exists(id: LinkId, exists: bool) -> OntologyError {
    if exists {
        OntologyError::LinkVersionConflict { id }
    } else {
        OntologyError::LinkMissing { id }
    }
}

fn object_write_failure_from_exists(id: ObjectId, exists: bool) -> OntologyError {
    if exists {
        OntologyError::ObjectVersionConflict { id }
    } else {
        OntologyError::ObjectMissing { id }
    }
}

fn revision_from_row(row: sqlx::mysql::MySqlRow) -> Result<PublishedRevision, OntologyError> {
    let id = revision_id_from_str(row.get("revision_id"))?;
    let schema = serde_json::from_str(&row.get::<String, _>("schema_json")).map_err(|source| {
        repository_error(MySqlOntologyRepositoryError::DecodeJson {
            kind: "schema",
            source,
        })
    })?;
    let revision = PublishedRevision { id, schema };
    validate_published_revision(&revision).map_err(|_| {
        repository_error(MySqlOntologyRepositoryError::InvalidPersisted {
            kind: "revision schema or identity",
        })
    })?;
    Ok(revision)
}

fn object_from_row(row: sqlx::mysql::MySqlRow) -> Result<ObjectRecord, OntologyError> {
    let values = serde_json::from_str::<Map<String, Value>>(&row.get::<String, _>("values_json"))
        .map_err(|source| {
        repository_error(MySqlOntologyRepositoryError::DecodeJson {
            kind: "object values",
            source,
        })
    })?;
    Ok(ObjectRecord {
        id: uuid_from_str(row.get("id"))?.into(),
        object_type_id: uuid_from_str(row.get("object_type_id"))?.into(),
        values,
        version: version_from_row(&row)?,
    })
}

fn link_from_row(row: sqlx::mysql::MySqlRow) -> Result<LinkRecord, OntologyError> {
    Ok(LinkRecord {
        id: uuid_from_str(row.get("id"))?.into(),
        link_type_id: uuid_from_str(row.get("link_type_id"))?.into(),
        source_object_id: uuid_from_str(row.get("source_object_id"))?.into(),
        target_object_id: uuid_from_str(row.get("target_object_id"))?.into(),
        version: version_from_row(&row)?,
    })
}

fn page<T>(
    rows: Vec<sqlx::mysql::MySqlRow>,
    limit: u32,
    decode: impl Fn(sqlx::mysql::MySqlRow) -> Result<T, OntologyError>,
    cursor: impl Fn(&T) -> Uuid,
) -> Result<Page<T>, OntologyError> {
    let has_more = rows.len() > limit as usize;
    let mut items = rows
        .into_iter()
        .map(decode)
        .collect::<Result<Vec<_>, _>>()?;
    if has_more {
        items.pop();
    }
    let next_after = has_more.then(|| items.last().map(cursor)).flatten();
    Ok(Page { items, next_after })
}

fn version_from_row(row: &sqlx::mysql::MySqlRow) -> Result<u64, OntologyError> {
    let value: u64 = row.get("version");
    Ok(value)
}

fn uuid_from_str(value: String) -> Result<Uuid, OntologyError> {
    Uuid::from_str(&value).map_err(|_| {
        repository_error(MySqlOntologyRepositoryError::InvalidPersisted { kind: "UUID" })
    })
}

fn revision_id_from_str(value: String) -> Result<RevisionId, OntologyError> {
    RevisionId::try_from(value).map_err(|_| {
        repository_error(MySqlOntologyRepositoryError::InvalidPersisted {
            kind: "revision id",
        })
    })
}

fn encode_values_error(source: serde_json::Error) -> OntologyError {
    repository_error(MySqlOntologyRepositoryError::DecodeJson {
        kind: "object values",
        source,
    })
}

fn sqlx_error(source: sqlx::Error) -> OntologyError {
    repository_error(MySqlOntologyRepositoryError::Sqlx(source))
}

fn repository_error(source: MySqlOntologyRepositoryError) -> OntologyError {
    OntologyError::Repository(Box::new(source))
}

fn map_object_delete_error(id: ObjectId, error: sqlx::Error) -> OntologyError {
    if matches!(&error, sqlx::Error::Database(database) if database.code().as_deref() == Some("1451"))
    {
        OntologyError::ObjectReferenced { id }
    } else {
        sqlx_error(error)
    }
}

#[cfg(test)]
mod tests {
    use std::{future::pending, sync::Arc};

    use tokio::{
        sync::Notify,
        time::{Duration, timeout},
    };
    use wyse_ontology::{ObjectId, OntologyError};

    use super::{InstanceWriteLock, object_write_failure_from_exists};

    #[test]
    fn object_write_failure_distinguishes_missing_from_version_conflict() {
        let id = ObjectId::new();

        assert!(
            matches!(object_write_failure_from_exists(id, false), OntologyError::ObjectMissing { id: actual } if actual == id)
        );
        assert!(
            matches!(object_write_failure_from_exists(id, true), OntologyError::ObjectVersionConflict { id: actual } if actual == id)
        );
    }

    #[tokio::test]
    #[ignore = "requires MySQL 8 started by the crate Makefile"]
    async fn cancelling_a_lock_holder_closes_its_mysql_session()
    -> Result<(), Box<dyn std::error::Error>> {
        let pool = sqlx::MySqlPool::connect(&std::env::var("DATABASE_URL")?).await?;
        let acquired = Arc::new(Notify::new());
        let holder = {
            let acquired = acquired.clone();
            let pool = pool.clone();
            tokio::spawn(async move {
                let _lock = InstanceWriteLock::acquire(&pool).await?;
                acquired.notify_one();
                pending::<()>().await;
                #[allow(unreachable_code)]
                Ok::<(), OntologyError>(())
            })
        };
        acquired.notified().await;
        holder.abort();
        assert!(
            holder
                .await
                .expect_err("holder is cancelled")
                .is_cancelled()
        );

        let lock = timeout(Duration::from_secs(2), InstanceWriteLock::acquire(&pool))
            .await
            .map_err(|_| "named lock remained held after task cancellation")??;
        lock.release().await?;
        Ok(())
    }
}
