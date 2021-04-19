//! This module contains lower-level primitives for visiting fields.

use serde_json::map::Map;
use serde_json::Value;
use std::fmt;
use tracing_core::field::{Field, Visit};

/// The visitor necessary to record values in GELF format.
#[derive(Debug)]
pub struct AdditionalFieldVisitor<'a> {
    object: &'a mut Map<String, Value>,
}

impl<'a> AdditionalFieldVisitor<'a> {
    /// Create a new [`AdditionalFieldVisitor`] from a [`Map`].
    pub fn new(object: &'a mut Map<String, Value>) -> Self {
        AdditionalFieldVisitor { object }
    }

    fn record_additional_value<V: Into<Value>>(&mut self, field: &str, value: V) {
        let new_key = format!("_{}", field);
        self.object.insert(new_key, value.into());
    }

    fn record_value<V: Into<Value>>(&mut self, field: &str, value: V) {
        self.object.insert(field.to_string(), value.into());
    }
}

impl<'a> Visit for AdditionalFieldVisitor<'a> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let value = format!("{:#?}", value);
        let field_name = field.name();
        match field_name {
            "target" => self.record_value(field_name, value),
            "module" => self.record_value(field_name, value),
            "file" => self.record_value(field_name, value),
            "message" => self.record_value(field_name, value),
            _ => self.record_additional_value(field_name, value),
        };
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        let field_name = field.name();
        match field_name {
            "target" => self.record_value(field_name, value),
            "module" => self.record_value(field_name, value),
            "file" => self.record_value(field_name, value),
            "message" => self.record_value(field_name, value),
            _ => self.record_additional_value(field_name, value),
        };
    }
}
