use std::fmt;

use crate::error::{Error, Result};

/// Location of a field inside a record, as a sequence of object keys.
///
/// The empty path designates the record itself, which is how a plain string
/// record is compared.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct FieldPath(Box<[Box<str>]>);

impl FieldPath {
    pub fn root() -> Self {
        Self::default()
    }

    /// Builds a path from explicit keys, which may contain dots.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSchema`] if a key is empty.
    pub fn from_keys<I, S>(keys: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<Box<str>>,
    {
        let keys: Box<[Box<str>]> = keys.into_iter().map(Into::into).collect();
        if keys.iter().any(|key| key.is_empty()) {
            return Err(Error::schema("field path keys must not be empty"));
        }
        Ok(Self(keys))
    }

    /// Parses a dotted path such as `params.retries`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSchema`] if the path or one of its keys is
    /// empty.
    pub fn parse(dotted: &str) -> Result<Self> {
        if dotted.is_empty() {
            return Err(Error::schema("field path must not be empty"));
        }
        Self::from_keys(dotted.split('.'))
    }

    pub fn keys(&self) -> &[Box<str>] {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// True when `self` equals `other` or contains it.
    pub fn is_prefix_of(&self, other: &Self) -> bool {
        other.0.starts_with(&self.0)
    }
}

impl fmt::Display for FieldPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_root() {
            return f.write_str("<record>");
        }
        for (index, key) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str(".")?;
            }
            f.write_str(key)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dotted_paths() {
        let path = FieldPath::parse("params.retries").unwrap();
        assert_eq!(path.keys().len(), 2);
        assert_eq!(path.to_string(), "params.retries");
    }

    #[test]
    fn rejects_empty_keys() {
        assert!(FieldPath::parse("").is_err());
        assert!(FieldPath::parse("a..b").is_err());
        assert!(FieldPath::from_keys(["a", ""]).is_err());
    }

    #[test]
    fn keys_may_contain_dots_when_explicit() {
        let path = FieldPath::from_keys(["a.b"]).unwrap();
        assert_eq!(path.keys().len(), 1);
    }

    #[test]
    fn prefix_relation() {
        let parent = FieldPath::parse("params").unwrap();
        let child = FieldPath::parse("params.retries").unwrap();
        assert!(parent.is_prefix_of(&child));
        assert!(!child.is_prefix_of(&parent));
        assert!(FieldPath::root().is_prefix_of(&child));
    }
}
