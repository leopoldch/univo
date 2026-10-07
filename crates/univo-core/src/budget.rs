//! Size and depth accounting of a record, charged before anything is copied.

use crate::error::{Error, Result};
use crate::path::FieldPath;
use crate::schema::Limits;
use crate::value::Value;

/// Bytes charged for every scalar or container, on top of the UTF-8 bytes
/// of strings and object keys.
const NODE_BYTES: usize = 8;

/// Running total of the selected size of one record.
#[derive(Debug)]
pub(crate) struct Budget {
    limits: Limits,
    used: usize,
}

enum Children<'a> {
    List(std::slice::Iter<'a, Value>),
    Map(std::slice::Iter<'a, (String, Value)>),
}

impl Budget {
    pub(crate) fn new(limits: Limits) -> Self {
        Self { limits, used: 0 }
    }

    /// Charges `value` and everything it contains.
    ///
    /// The traversal is iterative and holds at most `max_depth` frames, so
    /// neither untrusted nesting nor untrusted size can exhaust the stack or
    /// be traversed beyond the first limit it exceeds.
    pub(crate) fn charge_value(&mut self, path: &FieldPath, value: &Value) -> Result<()> {
        let mut frames: Vec<Children<'_>> = Vec::new();
        let mut next = Some(value);
        loop {
            if let Some(value) = next.take() {
                let text = if let Value::Str(text) = value {
                    text.len()
                } else {
                    0
                };
                self.charge(path, NODE_BYTES + text)?;
                let children = match value {
                    Value::List(items) => Some(Children::List(items.iter())),
                    Value::Map(map) => Some(Children::Map(map.entries().iter())),
                    _ => None,
                };
                if let Some(children) = children {
                    if frames.len() == self.limits.max_depth {
                        return Err(Error::nesting_too_deep(path, self.limits.max_depth));
                    }
                    frames.push(children);
                }
            }
            let Some(frame) = frames.last_mut() else {
                return Ok(());
            };
            match frame {
                Children::List(items) => match items.next() {
                    Some(item) => next = Some(item),
                    None => drop(frames.pop()),
                },
                Children::Map(entries) => match entries.next() {
                    Some((key, item)) => {
                        self.charge(path, key.len())?;
                        next = Some(item);
                    }
                    None => drop(frames.pop()),
                },
            }
        }
    }

    pub(crate) fn charge(&mut self, path: &FieldPath, bytes: usize) -> Result<()> {
        self.used = self.used.saturating_add(bytes);
        if self.used > self.limits.max_selected_bytes {
            return Err(Error::record_too_large(
                path,
                self.limits.max_selected_bytes,
                self.used,
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::value::Map;

    fn limits(max_depth: usize, max_selected_bytes: usize) -> Limits {
        Limits {
            max_depth,
            max_selected_bytes,
            ..Limits::default()
        }
    }

    fn nested(depth: usize) -> Value {
        let mut value = Value::Int(1);
        for _ in 0..depth {
            value = Value::Map(Map::new(vec![("k".to_owned(), value)]).unwrap());
        }
        value
    }

    fn resource(result: &Result<()>) -> Option<ErrorCode> {
        result.as_ref().err().map(Error::code)
    }

    #[test]
    fn depth_limit_is_inclusive() {
        let path = FieldPath::root();
        let mut budget = Budget::new(limits(16, usize::MAX));
        assert!(budget.charge_value(&path, &nested(16)).is_ok());
        let mut budget = Budget::new(limits(16, usize::MAX));
        assert_eq!(
            resource(&budget.charge_value(&path, &nested(17))),
            Some(ErrorCode::NestingTooDeep)
        );
    }

    #[test]
    fn counts_strings_keys_and_nodes() {
        let path = FieldPath::root();
        let value =
            Value::Map(Map::new(vec![("ab".to_owned(), Value::Str("xyz".into()))]).unwrap());
        // Object node, key "ab", string node with "xyz".
        let exact = 8 + 2 + 8 + 3;
        assert!(
            Budget::new(limits(16, exact))
                .charge_value(&path, &value)
                .is_ok()
        );
        assert_eq!(
            resource(&Budget::new(limits(16, exact - 1)).charge_value(&path, &value)),
            Some(ErrorCode::RecordTooLarge)
        );
    }

    #[test]
    fn size_accumulates_across_values() {
        let path = FieldPath::root();
        let mut budget = Budget::new(limits(16, 20));
        assert!(
            budget
                .charge_value(&path, &Value::Str("abc".into()))
                .is_ok()
        );
        assert_eq!(
            resource(&budget.charge_value(&path, &Value::Str("abc".into()))),
            Some(ErrorCode::RecordTooLarge)
        );
    }
}
