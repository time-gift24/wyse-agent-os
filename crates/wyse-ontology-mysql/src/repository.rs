//! MySQL implementation of the ontology repository contract.

use std::str::FromStr;

use async_trait::async_trait;
use serde_json::{Map, Value};
use sqlx::{Acquire, MySql, MySqlPool, Row, Transaction};
use uuid::Uuid;
use wyse_ontology::{
    LinkId, LinkRecord, LinkTypeId, NewLinkRecord, NewObjectRecord, ObjectId, ObjectRecord,
    ObjectTypeId, OntologyError, OntologyRepository, Page, PublishedRevision, RevisionId,
    SchemaValidationSnapshot, TagName, canonical_schema_bytes,
};

use crate::error::MySqlOntologyRepositoryError;

/// MySQL 8 repository for immutable revisions and shared ontology instances.
#[derive(Clone)]
pub struct SqlxOntologyRepository {
    pool: MySqlPool,
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
    async fn insert_revision(&self, revision: PublishedRevision) -> Result<(), OntologyError> {
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
        .execute(&self.pool)
        .await
        .map_err(sqlx_error)?;
        Ok(())
    }

    async fn get_revision(
        &self,
        id: &RevisionId,
    ) -> Result<Option<PublishedRevision>, OntologyError> {
        let row = sqlx::query(
            "SELECT revision_id, schema_json FROM ontology_revisions WHERE revision_id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_error)?;
        row.map(revision_from_row).transpose()
    }

    async fn list_revisions(&self) -> Result<Vec<PublishedRevision>, OntologyError> {
        sqlx::query("SELECT revision_id, schema_json FROM ontology_revisions ORDER BY created_at, revision_id")
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

    async fn schema_validation_snapshot(&self) -> Result<SchemaValidationSnapshot, OntologyError> {
        let mut connection = self.pool.acquire().await.map_err(sqlx_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *connection)
            .await
            .map_err(sqlx_error)?;
        let mut transaction = connection.begin().await.map_err(sqlx_error)?;
        let objects =
            sqlx::query("SELECT id, object_type_id, values_json, version FROM objects ORDER BY id")
                .fetch_all(&mut *transaction)
                .await
                .map_err(sqlx_error)?
                .into_iter()
                .map(object_from_row)
                .collect::<Result<Vec<_>, _>>()?;
        let links = sqlx::query(
            "SELECT id, link_type_id, source_object_id, target_object_id, version FROM links ORDER BY id",
        )
        .fetch_all(&mut *transaction)
        .await
        .map_err(sqlx_error)?
        .into_iter()
        .map(link_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        transaction.commit().await.map_err(sqlx_error)?;
        Ok(SchemaValidationSnapshot { objects, links })
    }

    async fn create_object(&self, object: NewObjectRecord) -> Result<ObjectRecord, OntologyError> {
        let values = serde_json::to_string(&object.values).map_err(encode_values_error)?;
        sqlx::query(
            "INSERT INTO objects (id, object_type_id, values_json, version) VALUES (?, ?, CAST(? AS JSON), 1)",
        )
        .bind(object.id.to_string())
        .bind(object.object_type_id.to_string())
        .bind(values)
        .execute(&self.pool)
        .await
        .map_err(sqlx_error)?;
        Ok(ObjectRecord {
            id: object.id,
            object_type_id: object.object_type_id,
            values: object.values,
            version: 1,
        })
    }

    async fn get_object(&self, id: ObjectId) -> Result<Option<ObjectRecord>, OntologyError> {
        sqlx::query("SELECT id, object_type_id, values_json, version FROM objects WHERE id = ?")
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
                    "SELECT id, object_type_id, values_json, version FROM objects \
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
                    "SELECT id, object_type_id, values_json, version FROM objects \
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

    async fn replace_object(&self, object: ObjectRecord) -> Result<ObjectRecord, OntologyError> {
        let values = serde_json::to_string(&object.values).map_err(encode_values_error)?;
        let result = sqlx::query(
            "UPDATE objects SET values_json = CAST(? AS JSON), version = version + 1, updated_at = UTC_TIMESTAMP(6) \
             WHERE id = ? AND version = ?",
        )
        .bind(values)
        .bind(object.id.to_string())
        .bind(object.version)
        .execute(&self.pool)
        .await
        .map_err(sqlx_error)?;
        if result.rows_affected() == 0 {
            return Err(object_write_failure(&self.pool, object.id).await?);
        }
        Ok(ObjectRecord {
            version: object.version + 1,
            ..object
        })
    }

    async fn delete_object(
        &self,
        id: ObjectId,
        version: u64,
        force: bool,
    ) -> Result<(), OntologyError> {
        if force {
            return delete_object_force(&self.pool, id, version).await;
        }
        let result = sqlx::query("DELETE FROM objects WHERE id = ? AND version = ?")
            .bind(id.to_string())
            .bind(version)
            .execute(&self.pool)
            .await
            .map_err(|error| map_object_delete_error(id, error))?;
        if result.rows_affected() == 0 {
            return Err(object_write_failure(&self.pool, id).await?);
        }
        Ok(())
    }

    async fn create_link(&self, link: NewLinkRecord) -> Result<LinkRecord, OntologyError> {
        sqlx::query(
            "INSERT INTO links (id, link_type_id, source_object_id, target_object_id, version) \
             VALUES (?, ?, ?, ?, 1)",
        )
        .bind(link.id.to_string())
        .bind(link.link_type_id.to_string())
        .bind(link.source_object_id.to_string())
        .bind(link.target_object_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(sqlx_error)?;
        Ok(LinkRecord {
            id: link.id,
            link_type_id: link.link_type_id,
            source_object_id: link.source_object_id,
            target_object_id: link.target_object_id,
            version: 1,
        })
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

    async fn replace_link(&self, link: LinkRecord) -> Result<LinkRecord, OntologyError> {
        let result = sqlx::query(
            "UPDATE links SET source_object_id = ?, target_object_id = ?, version = version + 1, updated_at = UTC_TIMESTAMP(6) \
             WHERE id = ? AND version = ?",
        )
        .bind(link.source_object_id.to_string())
        .bind(link.target_object_id.to_string())
        .bind(link.id.to_string())
        .bind(link.version)
        .execute(&self.pool)
        .await
        .map_err(sqlx_error)?;
        if result.rows_affected() == 0 {
            return Err(link_write_failure(&self.pool, link.id).await?);
        }
        Ok(LinkRecord {
            version: link.version + 1,
            ..link
        })
    }

    async fn delete_link(&self, id: LinkId, version: u64) -> Result<(), OntologyError> {
        let result = sqlx::query("DELETE FROM links WHERE id = ? AND version = ?")
            .bind(id.to_string())
            .bind(version)
            .execute(&self.pool)
            .await
            .map_err(sqlx_error)?;
        if result.rows_affected() == 0 {
            return Err(link_write_failure(&self.pool, id).await?);
        }
        Ok(())
    }

    async fn links_for_cardinality(
        &self,
        type_id: LinkTypeId,
        source: ObjectId,
        target: ObjectId,
        excluding: Option<LinkId>,
    ) -> Result<Vec<LinkRecord>, OntologyError> {
        let rows = match excluding {
            Some(excluding) => sqlx::query(
                "SELECT id, link_type_id, source_object_id, target_object_id, version FROM links \
                 WHERE link_type_id = ? AND (source_object_id = ? OR target_object_id = ?) AND id <> ?",
            )
            .bind(type_id.to_string())
            .bind(source.to_string())
            .bind(target.to_string())
            .bind(excluding.to_string())
            .fetch_all(&self.pool)
            .await,
            None => sqlx::query(
                "SELECT id, link_type_id, source_object_id, target_object_id, version FROM links \
                 WHERE link_type_id = ? AND (source_object_id = ? OR target_object_id = ?)",
            )
            .bind(type_id.to_string())
            .bind(source.to_string())
            .bind(target.to_string())
            .fetch_all(&self.pool)
            .await,
        }
        .map_err(sqlx_error)?;
        rows.into_iter().map(link_from_row).collect()
    }
}

async fn delete_object_force(
    pool: &MySqlPool,
    id: ObjectId,
    version: u64,
) -> Result<(), OntologyError> {
    let mut transaction = pool.begin().await.map_err(sqlx_error)?;
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

async fn object_write_failure(
    pool: &MySqlPool,
    id: ObjectId,
) -> Result<OntologyError, OntologyError> {
    Ok(object_write_failure_from_exists(
        id,
        object_exists_pool(pool, id).await?,
    ))
}

async fn link_write_failure(pool: &MySqlPool, id: LinkId) -> Result<OntologyError, OntologyError> {
    let exists = sqlx::query("SELECT 1 FROM links WHERE id = ?")
        .bind(id.to_string())
        .fetch_optional(pool)
        .await
        .map_err(sqlx_error)?
        .is_some();
    Ok(if exists {
        OntologyError::LinkVersionConflict { id }
    } else {
        OntologyError::LinkMissing { id }
    })
}

async fn object_exists_pool(pool: &MySqlPool, id: ObjectId) -> Result<bool, OntologyError> {
    sqlx::query("SELECT 1 FROM objects WHERE id = ?")
        .bind(id.to_string())
        .fetch_optional(pool)
        .await
        .map_err(sqlx_error)
        .map(|row| row.is_some())
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
    Ok(PublishedRevision { id, schema })
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
    use super::object_write_failure_from_exists;
    use wyse_ontology::{ObjectId, OntologyError};

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
}
