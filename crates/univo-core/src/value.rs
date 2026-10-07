use crate::error::{Error, Result};
use crate::path::FieldPath;

/// A record value. Types are never coerced: `Int(3)` and `Str("3")` differ,
/// and so do `Int(0)` and `Bool(false)`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(Finite),
    Str(String),
    List(Vec<Value>),
    Map(Map),
}

/// A finite `f64`; NaN and infinities are rejected at construction.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Finite(f64);

impl Finite {
    pub fn new(value: f64) -> Option<Self> {
        value.is_finite().then_some(Self(value))
    }

    pub fn get(self) -> f64 {
        self.0
    }
}

/// An object with unique keys, kept in input order.
///
/// Records are small, so lookups scan linearly; equality ignores key order
/// only after normalization.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Map(Vec<(String, Value)>);

impl Map {
    /// Builds a map.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] if a key appears twice.
    pub fn new(entries: Vec<(String, Value)>) -> Result<Self> {
        if let Some(key) = first_duplicate_key(&entries) {
            let path = FieldPath::from_keys([key]).unwrap_or_default();
            return Err(Error::record(&path, "duplicate key in object"));
        }
        Ok(Self(entries))
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0
            .iter()
            .find_map(|(candidate, value)| (candidate == key).then_some(value))
    }

    pub fn entries(&self) -> &[(String, Value)] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

fn first_duplicate_key(entries: &[(String, Value)]) -> Option<&str> {
    const LINEAR_SCAN_MAX: usize = 16;
    if entries.len() <= LINEAR_SCAN_MAX {
        return entries.iter().enumerate().find_map(|(index, (key, _))| {
            entries[..index]
                .iter()
                .any(|(other, _)| other == key)
                .then_some(key.as_str())
        });
    }
    let mut seen = std::collections::HashSet::with_capacity(entries.len());
    entries
        .iter()
        .find_map(|(key, _)| (!seen.insert(key.as_str())).then_some(key.as_str()))
}

/// Result of looking up a path: absent and null are different states.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Lookup<'a> {
    Absent,
    Present(&'a Value),
}

impl Value {
    pub fn float(value: f64) -> Option<Self> {
        Finite::new(value).map(Self::Float)
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::Int(_) => "integer",
            Self::Float(_) => "float",
            Self::Str(_) => "string",
            Self::List(_) => "list",
            Self::Map(_) => "object",
        }
    }

    /// Follows `path` through nested objects.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when the path traverses a value that
    /// is not an object: a schema mistake must not read as missing data.
    pub fn lookup(&self, path: &FieldPath) -> Result<Lookup<'_>> {
        let mut current = self;
        for (depth, key) in path.keys().iter().enumerate() {
            let Self::Map(map) = current else {
                let parent = FieldPath::from_keys(path.keys()[..depth].iter().map(AsRef::as_ref))?;
                return Err(Error::record(
                    &parent,
                    format!("expected an object, found {}", current.type_name()),
                ));
            };
            match map.get(key) {
                Some(value) => current = value,
                None => return Ok(Lookup::Absent),
            }
        }
        Ok(Lookup::Present(current))
    }

    /// Number of nested lists and objects, counting this value.
    pub fn depth(&self) -> usize {
        match self {
            Self::List(items) => 1 + items.iter().map(Self::depth).max().unwrap_or(0),
            Self::Map(map) => {
                1 + map
                    .0
                    .iter()
                    .map(|(_, value)| value.depth())
                    .max()
                    .unwrap_or(0)
            }
            _ => 0,
        }
    }

    /// Approximate size used for resource limits: UTF-8 bytes of strings
    /// and keys, plus eight bytes per scalar or container.
    pub fn selected_bytes(&self) -> usize {
        8 + match self {
            Self::Str(text) => text.len(),
            Self::List(items) => items.iter().map(Self::selected_bytes).sum(),
            Self::Map(map) => map
                .0
                .iter()
                .map(|(key, value)| key.len() + value.selected_bytes())
                .sum(),
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(entries: Vec<(&str, Value)>) -> Value {
        Value::Map(
            Map::new(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value))
                    .collect(),
            )
            .unwrap(),
        )
    }

    #[test]
    fn rejects_non_finite_floats() {
        assert!(Value::float(f64::NAN).is_none());
        assert!(Value::float(f64::INFINITY).is_none());
        assert_eq!(Value::float(0.5), Some(Value::Float(Finite(0.5))));
    }

    #[test]
    fn rejects_duplicate_keys() {
        let entries = vec![("a".to_owned(), Value::Null), ("a".to_owned(), Value::Null)];
        assert!(Map::new(entries).is_err());
    }

    #[test]
    fn rejects_duplicate_keys_in_large_objects() {
        let mut entries: Vec<_> = (0..100).map(|i| (i.to_string(), Value::Null)).collect();
        assert!(Map::new(entries.clone()).is_ok());
        entries.push(("42".to_owned(), Value::Null));
        assert!(Map::new(entries).is_err());
    }

    #[test]
    fn lookup_distinguishes_absent_and_null() {
        let record = object(vec![("a", Value::Null)]);
        let a = FieldPath::parse("a").unwrap();
        let b = FieldPath::parse("b").unwrap();
        assert_eq!(record.lookup(&a).unwrap(), Lookup::Present(&Value::Null));
        assert_eq!(record.lookup(&b).unwrap(), Lookup::Absent);
    }

    #[test]
    fn lookup_through_scalar_is_an_error() {
        let record = object(vec![("a", Value::Int(1))]);
        let error = record
            .lookup(&FieldPath::parse("a.b").unwrap())
            .unwrap_err();
        assert!(matches!(error, Error::InvalidRecord { .. }));
    }

    #[test]
    fn depth_counts_containers() {
        assert_eq!(Value::Int(1).depth(), 0);
        assert_eq!(object(vec![("a", Value::List(vec![]))]).depth(), 2);
    }
}
