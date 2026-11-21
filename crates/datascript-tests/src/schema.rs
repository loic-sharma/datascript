//! The `schema:` header and the mock schema provider built from it.
//!
//! `schema: orders(orderId: string, amount: real); 'data/x.csv'(n: long)` declares the schemas a
//! `check` or `explain` case sees without reading any file (TESTING.md, "Header keys"). Type names
//! are DataScript types (LANGUAGE.md §5); the harness maps them to the canonical Arrow types
//! itself so the compiler is not involved in reading test files.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema, SchemaRef, TimeUnit};
use datascript_compiler::{SchemaLookup, SchemaProvider, SourceError, SourceRef};
use datascript_diagnostics::did_you_mean;

/// DataScript type names accepted in `schema:`.
pub const TYPE_NAMES: &[&str] = &[
    "bool", "int", "long", "real", "decimal", "string", "datetime", "timespan", "guid", "dynamic",
];

const EXTENSION_NAME_KEY: &str = "ARROW:extension:name";

/// Schemas declared by a case's `schema:` header, keyed by source name.
///
/// The name is a host table name (`orders`) or a quoted path (`'../data/orders.csv'`, stored
/// without quotes). It implements the compiler's [`SchemaProvider`]: a source it does not declare
/// is not found, with the declared file names as nearby files.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MockSchemaProvider {
    tables: BTreeMap<String, SchemaRef>,
}

impl MockSchemaProvider {
    /// The schema declared for `name`, if any.
    pub fn schema_of(&self, name: &str) -> Option<SchemaRef> {
        self.tables.get(name).cloned()
    }

    /// Declared source names, sorted.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.tables.keys().map(String::as_str)
    }
}

impl SchemaProvider for MockSchemaProvider {
    fn schema_of(&self, source: &SourceRef) -> SchemaLookup {
        let SourceRef::Path(path) = source;
        match self.tables.get(path) {
            Some(schema) => SchemaLookup::Ready(schema.clone()),
            None => {
                let mut nearby: Vec<String> = self
                    .names()
                    .map(|name| name.rsplit(['/', '\\']).next().unwrap_or(name).to_string())
                    .collect();
                nearby.sort();
                nearby.dedup();
                SchemaLookup::Failed(SourceError::NotFound { nearby })
            }
        }
    }
}

/// The canonical Arrow field for a DataScript type (LANGUAGE.md §5).
pub fn canonical_field(name: &str, ds_type: &str) -> Option<Field> {
    let (data_type, extension) = match ds_type {
        "bool" => (DataType::Boolean, None),
        "int" => (DataType::Int32, None),
        "long" => (DataType::Int64, None),
        "real" => (DataType::Float64, None),
        "decimal" => (DataType::Decimal128(38, 18), None),
        "string" => (DataType::Utf8, None),
        "datetime" => (
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            None,
        ),
        "timespan" => (DataType::Duration(TimeUnit::Microsecond), None),
        "guid" => (DataType::FixedSizeBinary(16), Some("arrow.uuid")),
        "dynamic" => (DataType::Utf8, Some("arrow.json")),
        _ => return None,
    };
    let field = Field::new(name, data_type, true);
    Some(match extension {
        Some(ext) => field.with_metadata(HashMap::from([(
            EXTENSION_NAME_KEY.to_string(),
            ext.to_string(),
        )])),
        None => field,
    })
}

/// Parses the value of a `schema:` header. Errors are messages without a line number.
pub fn parse(value: &str) -> Result<MockSchemaProvider, String> {
    let mut p = Cursor {
        chars: value.chars().collect(),
        pos: 0,
    };
    let mut provider = MockSchemaProvider::default();
    loop {
        p.skip_ws();
        if p.at_end() {
            break;
        }
        let table = p.table_name()?;
        p.skip_ws();
        p.expect('(', &format!("after table name `{table}`"))?;
        let mut fields: Vec<Field> = Vec::new();
        p.skip_ws();
        if !p.eat(')') {
            loop {
                p.skip_ws();
                let column = p.column_name()?;
                p.skip_ws();
                p.expect(':', &format!("after column `{column}`"))?;
                p.skip_ws();
                let ty = p.word();
                if ty.is_empty() {
                    return Err(format!(
                        "`schema`: expected a type for column `{column}`, {}",
                        p.found()
                    ));
                }
                let field = canonical_field(&column, &ty).ok_or_else(|| {
                    let hint = match did_you_mean(&ty, TYPE_NAMES.iter().copied()).single() {
                        Some(t) => format!(" (did you mean `{t}`?)"),
                        None => format!(" (types: {})", TYPE_NAMES.join(", ")),
                    };
                    format!("`schema`: unknown type `{ty}` for column `{column}`{hint}")
                })?;
                if fields.iter().any(|f| f.name() == &column) {
                    return Err(format!(
                        "`schema`: column `{column}` appears twice in `{table}`"
                    ));
                }
                fields.push(field);
                p.skip_ws();
                if p.eat(')') {
                    break;
                }
                p.expect(',', &format!("after column `{column}`"))?;
            }
        }
        if provider.tables.contains_key(&table) {
            return Err(format!("`schema`: table `{table}` is declared twice"));
        }
        provider.tables.insert(table, Arc::new(Schema::new(fields)));
        p.skip_ws();
        if p.at_end() {
            break;
        }
        p.expect(';', "between tables")?;
    }
    if provider.tables.is_empty() {
        return Err("`schema` needs at least one table, e.g. `orders(id: long)`".to_string());
    }
    Ok(provider)
}

struct Cursor {
    chars: Vec<char>,
    pos: usize,
}

impl Cursor {
    fn at_end(&self) -> bool {
        self.pos >= self.chars.len()
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn found(&self) -> String {
        match self.peek() {
            Some(c) => format!("found `{c}` at column {}", self.pos + 1),
            None => "found the end of the header".to_string(),
        }
    }

    fn expect(&mut self, c: char, context: &str) -> Result<(), String> {
        if self.eat(c) {
            Ok(())
        } else {
            Err(format!(
                "`schema`: expected `{c}` {context}, {}",
                self.found()
            ))
        }
    }

    fn word(&mut self) -> String {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_alphanumeric() || c == '_') {
            self.pos += 1;
        }
        self.chars[start..self.pos].iter().collect()
    }

    fn quoted(&mut self) -> Result<String, String> {
        let quote = self.peek().expect("caller checked for a quote");
        let start = self.pos;
        self.pos += 1;
        let mut out = String::new();
        loop {
            match self.peek() {
                None => {
                    return Err(format!(
                        "`schema`: unterminated quoted name starting at column {}",
                        start + 1
                    ));
                }
                Some(c) if c == quote => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(c) => {
                    out.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    /// An identifier or a quoted path.
    fn table_name(&mut self) -> Result<String, String> {
        if matches!(self.peek(), Some('\'' | '"')) {
            return self.quoted();
        }
        let name = self.word();
        if name.is_empty() {
            return Err(format!("`schema`: expected a table name, {}", self.found()));
        }
        Ok(name)
    }

    /// An identifier or `['quoted name']` / `["quoted name"]` (LANGUAGE.md §1).
    fn column_name(&mut self) -> Result<String, String> {
        if self.eat('[') {
            if !matches!(self.peek(), Some('\'' | '"')) {
                return Err(format!(
                    "`schema`: expected a quoted name after `[`, {}",
                    self.found()
                ));
            }
            let name = self.quoted()?;
            self.expect(']', "after the quoted column name")?;
            return Ok(name);
        }
        let name = self.word();
        if name.is_empty() {
            return Err(format!(
                "`schema`: expected a column name, {}",
                self.found()
            ));
        }
        Ok(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tables_and_types() {
        let p = parse(
            "orders(orderId: string, amount: real, ts: datetime); \
             '../data/x.csv'(['order id']: long, j: dynamic); empty()",
        )
        .unwrap();
        assert_eq!(
            p.names().collect::<Vec<_>>(),
            ["../data/x.csv", "empty", "orders"]
        );
        let orders = p.schema_of("orders").unwrap();
        assert_eq!(orders.field(0).data_type(), &DataType::Utf8);
        assert_eq!(orders.field(1).data_type(), &DataType::Float64);
        assert_eq!(
            orders.field(2).data_type(),
            &DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
        );
        let x = p.schema_of("../data/x.csv").unwrap();
        assert_eq!(x.field(0).name(), "order id");
        assert_eq!(
            x.field(1).metadata().get(EXTENSION_NAME_KEY).unwrap(),
            "arrow.json"
        );
        assert!(p.schema_of("empty").unwrap().fields().is_empty());
        assert!(p.schema_of("missing").is_none());
    }

    #[test]
    fn every_type_name_maps() {
        for t in TYPE_NAMES {
            assert!(canonical_field("c", t).is_some(), "{t}");
        }
    }

    #[test]
    fn errors() {
        let e = |s| parse(s).unwrap_err();
        assert_eq!(
            e("orders"),
            "`schema`: expected `(` after table name `orders`, found the end of the header"
        );
        assert_eq!(
            e("t(a long)"),
            "`schema`: expected `:` after column `a`, found `l` at column 5"
        );
        assert_eq!(
            e("t(a: long b: real)"),
            "`schema`: expected `,` after column `a`, found `b` at column 11"
        );
        assert_eq!(
            e("t(a: long, a: real)"),
            "`schema`: column `a` appears twice in `t`"
        );
        assert_eq!(
            e("t(a: long) u(b: real)"),
            "`schema`: expected `;` between tables, found `u` at column 12"
        );
        assert_eq!(e("t(); t()"), "`schema`: table `t` is declared twice");
        assert_eq!(
            e("'x.csv(a: long)"),
            "`schema`: unterminated quoted name starting at column 1"
        );
        assert_eq!(
            e("t(a: thing)"),
            "`schema`: unknown type `thing` for column `a` (types: bool, int, long, real, decimal, string, datetime, timespan, guid, dynamic)"
        );
    }
}
