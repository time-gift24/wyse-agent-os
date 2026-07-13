import type { OntologyGraph, SchemaDocument } from "~/lib/ontology-api"

const PERSON = "00000000-0000-4000-8000-000000000001"
const COMPANY = "00000000-0000-4000-8000-000000000002"
const PROJECT = "00000000-0000-4000-8000-000000000003"
const DOCUMENT = "00000000-0000-4000-8000-000000000004"

export const DEMO_ONTOLOGY: {
  graph: OntologyGraph
  schema: SchemaDocument
} = {
  schema: {
    schema_version: 1,
    object_types: [
      {
        id: PERSON,
        name: "Person",
        description: "A person participating in the organization.",
        properties: [
          {
            id: "00000000-0000-4000-8000-000000000101",
            name: "name",
            description: "Display name",
            value_type: "string",
            required: true,
          },
          {
            id: "00000000-0000-4000-8000-000000000102",
            name: "email",
            description: "Contact email",
            value_type: "string",
            required: false,
          },
          {
            id: "00000000-0000-4000-8000-000000000103",
            name: "created_at",
            description: "Creation time",
            value_type: "datetime",
            required: true,
          },
        ],
      },
      {
        id: COMPANY,
        name: "Company",
        description: "An organization employing people.",
        properties: [
          {
            id: "00000000-0000-4000-8000-000000000201",
            name: "name",
            description: "Registered name",
            value_type: "string",
            required: true,
          },
        ],
      },
      {
        id: PROJECT,
        name: "Project",
        description: "A coordinated body of work.",
        properties: [
          {
            id: "00000000-0000-4000-8000-000000000301",
            name: "title",
            description: "Project title",
            value_type: "string",
            required: true,
          },
          {
            id: "00000000-0000-4000-8000-000000000302",
            name: "active",
            description: "Whether work is active",
            value_type: "boolean",
            required: true,
          },
        ],
      },
      {
        id: DOCUMENT,
        name: "Document",
        description: "A document produced by a project.",
        properties: [
          {
            id: "00000000-0000-4000-8000-000000000401",
            name: "title",
            description: "Document title",
            value_type: "string",
            required: true,
          },
        ],
      },
    ],
    link_types: [
      {
        id: "00000000-0000-4000-8000-000000000501",
        name: "works_at",
        description: "A person works at a company.",
        source_object_type_id: PERSON,
        target_object_type_id: COMPANY,
        cardinality: "many_to_one",
      },
      {
        id: "00000000-0000-4000-8000-000000000502",
        name: "owns",
        description: "A person owns projects.",
        source_object_type_id: PERSON,
        target_object_type_id: PROJECT,
        cardinality: "one_to_many",
      },
      {
        id: "00000000-0000-4000-8000-000000000503",
        name: "produces",
        description: "A project produces documents.",
        source_object_type_id: PROJECT,
        target_object_type_id: DOCUMENT,
        cardinality: "one_to_many",
      },
    ],
  },
  graph: {
    schema_ref: { kind: "tag", name: "online" },
    nodes: [
      { id: PERSON, label: "Person", property_count: 3 },
      { id: COMPANY, label: "Company", property_count: 1 },
      { id: PROJECT, label: "Project", property_count: 2 },
      { id: DOCUMENT, label: "Document", property_count: 1 },
    ],
    edges: [
      {
        id: "00000000-0000-4000-8000-000000000501",
        label: "works_at",
        source: PERSON,
        target: COMPANY,
        cardinality: "many_to_one",
      },
      {
        id: "00000000-0000-4000-8000-000000000502",
        label: "owns",
        source: PERSON,
        target: PROJECT,
        cardinality: "one_to_many",
      },
      {
        id: "00000000-0000-4000-8000-000000000503",
        label: "produces",
        source: PROJECT,
        target: DOCUMENT,
        cardinality: "one_to_many",
      },
    ],
  },
}
