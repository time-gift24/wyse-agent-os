//! JSON value validation against object property definitions.

use chrono::DateTime;
use serde_json::{Map, Value};

use crate::{OntologyError, PropertyType, ValueType};

/// Validates an object value document against ordered property definitions.
///
/// # Errors
///
/// Returns [`OntologyError::ValueInvalid`] when a property is unknown, required
/// data is absent, or a value has the wrong JSON representation.
pub fn validate_object_values(
    properties: &[PropertyType],
    values: &Map<String, Value>,
) -> Result<(), OntologyError> {
    let mut diagnostics = Vec::new();

    for name in values.keys() {
        if !properties.iter().any(|property| property.name == *name) {
            diagnostics.push(format!("unknown property: {name}"));
        }
    }

    for property in properties {
        match values.get(&property.name) {
            None if property.required => {
                diagnostics.push(format!("missing required property: {}", property.name))
            }
            None => {}
            Some(value) if !matches_value_type(value, property.value_type) => {
                diagnostics.push(format!(
                    "property {} must be {}",
                    property.name,
                    value_type_name(property.value_type)
                ))
            }
            Some(_) => {}
        }
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(OntologyError::ValueInvalid { diagnostics })
    }
}

fn matches_value_type(value: &Value, value_type: ValueType) -> bool {
    match value_type {
        ValueType::String => value.is_string(),
        ValueType::Integer => value.as_i64().is_some() || value.as_u64().is_some(),
        ValueType::Number => value.is_number(),
        ValueType::Boolean => value.is_boolean(),
        ValueType::Datetime => value
            .as_str()
            .is_some_and(|value| DateTime::parse_from_rfc3339(value).is_ok()),
        ValueType::Json => true,
    }
}

fn value_type_name(value_type: ValueType) -> &'static str {
    match value_type {
        ValueType::String => "a string",
        ValueType::Integer => "an integer",
        ValueType::Number => "a number",
        ValueType::Boolean => "a boolean",
        ValueType::Datetime => "an RFC 3339 datetime string",
        ValueType::Json => "JSON",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, json};

    use super::validate_object_values;
    use crate::{OntologyError, PropertyType, PropertyTypeId, ValueType};

    fn required_property(name: &str, value_type: ValueType) -> PropertyType {
        PropertyType {
            id: PropertyTypeId::new(),
            name: name.to_owned(),
            description: String::new(),
            value_type,
            required: true,
        }
    }

    #[test]
    fn required_datetime_must_be_rfc3339_string() {
        let property = required_property("created_at", ValueType::Datetime);
        let values = Map::from_iter([("created_at".to_owned(), json!(42))]);

        assert!(matches!(
            validate_object_values(&[property], &values),
            Err(OntologyError::ValueInvalid { .. })
        ));
    }
}
