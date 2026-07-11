use wyse_ontology::{
    ObjectType, ObjectTypeId, PublishedRevision, RevisionId, SchemaDocument, TagName,
};
use wyse_ontology_mysql::SqlxOntologyRepository;

fn published_revision() -> PublishedRevision {
    PublishedRevision {
        id: RevisionId::try_from("a".repeat(64)).expect("valid revision id"),
        schema: SchemaDocument {
            schema_version: 1,
            object_types: vec![ObjectType {
                id: ObjectTypeId::new(),
                name: "person".to_owned(),
                description: String::new(),
                properties: Vec::new(),
            }],
            link_types: Vec::new(),
        },
    }
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
