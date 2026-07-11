CREATE TABLE ontology_revisions (
    revision_id CHAR(64) CHARACTER SET ascii NOT NULL,
    schema_json JSON NOT NULL,
    schema_format_version INT UNSIGNED NOT NULL,
    created_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    PRIMARY KEY (revision_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE ontology_tags (
    name VARCHAR(64) NOT NULL,
    revision_id CHAR(64) CHARACTER SET ascii NOT NULL,
    updated_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    PRIMARY KEY (name),
    CONSTRAINT ontology_tags_revision_id_fk
        FOREIGN KEY (revision_id) REFERENCES ontology_revisions (revision_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE objects (
    id CHAR(36) CHARACTER SET ascii NOT NULL,
    object_type_id CHAR(36) CHARACTER SET ascii NOT NULL,
    values_json JSON NOT NULL,
    version BIGINT UNSIGNED NOT NULL,
    created_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    updated_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    PRIMARY KEY (id),
    KEY objects_object_type_id_idx (object_type_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE links (
    id CHAR(36) CHARACTER SET ascii NOT NULL,
    link_type_id CHAR(36) CHARACTER SET ascii NOT NULL,
    source_object_id CHAR(36) CHARACTER SET ascii NOT NULL,
    target_object_id CHAR(36) CHARACTER SET ascii NOT NULL,
    version BIGINT UNSIGNED NOT NULL,
    created_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    updated_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    PRIMARY KEY (id),
    KEY links_link_type_id_idx (link_type_id),
    KEY links_source_object_id_idx (source_object_id),
    KEY links_target_object_id_idx (target_object_id),
    CONSTRAINT links_source_object_id_fk FOREIGN KEY (source_object_id) REFERENCES objects (id),
    CONSTRAINT links_target_object_id_fk FOREIGN KEY (target_object_id) REFERENCES objects (id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
