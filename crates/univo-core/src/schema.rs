use crate::error::{Error, Result};
use crate::path::FieldPath;

/// Fields to compare, record identity, partitions and resource limits.
///
/// A schema is validated once at construction; preparing records relies on
/// its invariants.
#[derive(Debug, Clone, PartialEq)]
pub struct Schema {
    id_field: Option<FieldPath>,
    partition_by: Vec<FieldPath>,
    fields: Vec<Field>,
    limits: Limits,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub path: FieldPath,
    pub kind: FieldKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    Text(Text),
    Exact(Exact),
}

/// Free text, compared lexically and/or semantically.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Text {
    pub mode: TextMode,
    pub weight: f32,
    /// Unicode full case folding before comparison.
    pub case_fold: bool,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextMode {
    Lexical,
    Semantic,
    Hybrid,
}

/// A value compared for typed equality after its normalizer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Exact {
    pub normalizer: ExactNormalizer,
    pub weight: f32,
    pub required: bool,
    pub conflict: Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExactNormalizer {
    /// Unicode NFC on strings, nothing else.
    Identity,
    /// NFC, ASCII-only lowercasing and removal of one trailing dot.
    Hostname,
}

/// Effect of two present, different values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    /// Lowers the score.
    Evidence,
    /// Forbids the match (veto).
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_fields: usize,
    pub max_depth: usize,
    pub max_selected_bytes: usize,
}

impl Default for Text {
    fn default() -> Self {
        Self {
            mode: TextMode::Hybrid,
            weight: 1.0,
            case_fold: true,
            required: false,
        }
    }
}

impl Default for Exact {
    fn default() -> Self {
        Self {
            normalizer: ExactNormalizer::Identity,
            weight: 1.0,
            required: false,
            conflict: Conflict::Evidence,
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_fields: 64,
            max_depth: 16,
            max_selected_bytes: 1 << 20,
        }
    }
}

impl FieldKind {
    pub fn weight(&self) -> f32 {
        match self {
            Self::Text(text) => text.weight,
            Self::Exact(exact) => exact.weight,
        }
    }

    pub fn is_required(&self) -> bool {
        match self {
            Self::Text(text) => text.required,
            Self::Exact(exact) => exact.required,
        }
    }
}

impl Schema {
    /// Schema for plain string records, compared as one text field.
    pub fn strings() -> Self {
        Self {
            id_field: None,
            partition_by: Vec::new(),
            fields: vec![Field {
                path: FieldPath::root(),
                kind: FieldKind::Text(Text::default()),
            }],
            limits: Limits::default(),
        }
    }

    pub fn builder() -> SchemaBuilder {
        SchemaBuilder::default()
    }

    pub fn id_field(&self) -> Option<&FieldPath> {
        self.id_field.as_ref()
    }

    pub fn partition_by(&self) -> &[FieldPath] {
        &self.partition_by
    }

    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// True for [`Schema::strings`]: records are strings, not objects.
    pub fn compares_whole_record(&self) -> bool {
        self.fields.len() == 1 && self.fields[0].path.is_root()
    }
}

#[derive(Debug, Clone, Default)]
pub struct SchemaBuilder {
    id_field: Option<FieldPath>,
    partition_by: Vec<FieldPath>,
    fields: Vec<Field>,
    limits: Limits,
}

impl SchemaBuilder {
    #[must_use]
    pub fn id_field(mut self, path: FieldPath) -> Self {
        self.id_field = Some(path);
        self
    }

    #[must_use]
    pub fn partition_by(mut self, path: FieldPath) -> Self {
        self.partition_by.push(path);
        self
    }

    #[must_use]
    pub fn text(mut self, path: FieldPath, text: Text) -> Self {
        self.fields.push(Field {
            path,
            kind: FieldKind::Text(text),
        });
        self
    }

    #[must_use]
    pub fn exact(mut self, path: FieldPath, exact: Exact) -> Self {
        self.fields.push(Field {
            path,
            kind: FieldKind::Exact(exact),
        });
        self
    }

    #[must_use]
    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Validates the schema.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSchema`] when no field is selected, too many
    /// are, a weight is not a positive finite number, a limit is zero, or two
    /// paths among the identifier, partition keys and fields overlap.
    pub fn build(self) -> Result<Schema> {
        let limits = self.limits;
        if limits.max_fields == 0 || limits.max_depth == 0 || limits.max_selected_bytes == 0 {
            return Err(Error::schema("limits must be positive"));
        }
        if self.fields.is_empty() {
            return Err(Error::schema("at least one field must be compared"));
        }
        if self.fields.len() > limits.max_fields {
            return Err(Error::schema(format!(
                "{} fields selected, above the limit of {}",
                self.fields.len(),
                limits.max_fields
            )));
        }
        for field in &self.fields {
            let weight = field.kind.weight();
            if !(weight.is_finite() && weight > 0.0) {
                return Err(Error::schema(format!(
                    "weight of {} must be a positive finite number",
                    field.path
                )));
            }
            if field.path.is_root() {
                return Err(Error::schema(
                    "the whole record can only be compared with Schema::strings",
                ));
            }
        }

        let mut paths: Vec<(&FieldPath, &str)> = Vec::new();
        paths.extend(self.id_field.iter().map(|path| (path, "identifier")));
        paths.extend(self.partition_by.iter().map(|path| (path, "partition key")));
        paths.extend(
            self.fields
                .iter()
                .map(|field| (&field.path, "compared field")),
        );
        for (index, (path, role)) in paths.iter().enumerate() {
            if path.is_root() {
                return Err(Error::schema(format!("the {role} path must not be empty")));
            }
            for (other, other_role) in &paths[..index] {
                if path.is_prefix_of(other) || other.is_prefix_of(path) {
                    return Err(Error::schema(format!(
                        "{other_role} {other} and {role} {path} overlap"
                    )));
                }
            }
        }

        Ok(Schema {
            id_field: self.id_field,
            partition_by: self.partition_by,
            fields: self.fields,
            limits,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(dotted: &str) -> FieldPath {
        FieldPath::parse(dotted).unwrap()
    }

    #[test]
    fn builds_a_valid_schema() {
        let schema = Schema::builder()
            .id_field(path("id"))
            .partition_by(path("tenant_id"))
            .text(path("name"), Text::default())
            .exact(
                path("domain"),
                Exact {
                    normalizer: ExactNormalizer::Hostname,
                    ..Exact::default()
                },
            )
            .build()
            .unwrap();
        assert_eq!(schema.fields().len(), 2);
        assert!(!schema.compares_whole_record());
        assert!(Schema::strings().compares_whole_record());
    }

    #[test]
    fn rejects_empty_schema() {
        assert!(Schema::builder().build().is_err());
    }

    #[test]
    fn rejects_invalid_weights() {
        for weight in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let text = Text {
                weight,
                ..Text::default()
            };
            assert!(
                Schema::builder().text(path("name"), text).build().is_err(),
                "weight {weight} was accepted"
            );
        }
    }

    #[test]
    fn rejects_overlapping_paths() {
        let overlapping = [
            Schema::builder()
                .text(path("name"), Text::default())
                .text(path("name"), Text::default()),
            Schema::builder()
                .exact(path("params"), Exact::default())
                .exact(path("params.retries"), Exact::default()),
            Schema::builder()
                .id_field(path("id"))
                .exact(path("id"), Exact::default()),
            Schema::builder()
                .partition_by(path("tenant"))
                .text(path("tenant.name"), Text::default()),
        ];
        for builder in overlapping {
            assert!(builder.build().is_err());
        }
    }

    #[test]
    fn rejects_too_many_fields() {
        let mut builder = Schema::builder();
        for index in 0..65 {
            builder = builder.text(path(&format!("f{index}")), Text::default());
        }
        assert!(builder.build().is_err());
    }

    #[test]
    fn rejects_root_field_in_object_schema() {
        assert!(
            Schema::builder()
                .text(FieldPath::root(), Text::default())
                .build()
                .is_err()
        );
    }
}
