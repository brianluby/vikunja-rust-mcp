//! Adjustments to the JSON schemas published for MCP tools.

use schemars::Schema;
use serde_json::Value;

/// Removes schemars' Rust-specific unsigned integer `format` markers
/// (`uint`, `uint8`, ..., `uint128`). They are not JSON Schema or OpenAPI
/// formats, so strict MCP clients log an "unknown format ignored" warning
/// for every occurrence. The `minimum: 0` bound schemars also emits is kept,
/// so no validation information is lost.
///
/// Apply to any schema-published type containing unsigned integers:
/// `#[schemars(transform = RecursiveTransform(strip_unsigned_formats))]`.
pub fn strip_unsigned_formats(schema: &mut Schema) {
    if let Some(object) = schema.as_object_mut()
        && matches!(
            object.get("format").and_then(Value::as_str),
            Some("uint" | "uint8" | "uint16" | "uint32" | "uint64" | "uint128")
        )
    {
        object.remove("format");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::transform::RecursiveTransform;
    use schemars::{JsonSchema, schema_for};

    // Only the generated schema matters; the fields are never read.
    #[allow(dead_code)]
    #[derive(JsonSchema)]
    #[schemars(transform = RecursiveTransform(strip_unsigned_formats))]
    struct Probe {
        page: u32,
        count: usize,
        size: Option<u64>,
        status: Option<u16>,
        id: i64,
    }

    #[test]
    fn unsigned_formats_are_removed_but_bounds_and_int64_stay() {
        let schema = serde_json::to_value(schema_for!(Probe)).unwrap();
        assert!(
            !schema.to_string().contains("uint"),
            "unsigned formats should be stripped: {schema}"
        );
        // The zero lower bound survives for unsigned fields.
        assert_eq!(schema["properties"]["page"]["minimum"], 0);
        assert_eq!(schema["properties"]["count"]["minimum"], 0);
        // Recognized signed formats are untouched.
        assert_eq!(schema["properties"]["id"]["format"], "int64");
    }
}
