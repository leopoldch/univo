//! Model references written by users, as defined in
//! `specs/02-python-api.md` §5.5.

use std::fmt;
use std::path::PathBuf;

use univo_core::{Error, Result};

/// A Hugging Face repository, `organization/name`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Repository {
    pub organization: String,
    pub name: String,
}

/// Where a model comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelSpec {
    /// The default model of this release, pinned in [`crate::registry`].
    Default,
    /// A Hugging Face repository, optionally at a revision (branch, tag or
    /// commit); an absent revision means the default branch.
    Hub {
        repository: Repository,
        revision: Option<String>,
    },
    /// A local directory holding the model files.
    Local(PathBuf),
}

impl fmt::Display for Repository {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.organization, self.name)
    }
}

impl fmt::Display for ModelSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => f.write_str("default"),
            Self::Hub {
                repository,
                revision: Some(revision),
            } => write!(f, "{repository}@{revision}"),
            Self::Hub {
                repository,
                revision: None,
            } => write!(f, "{repository}"),
            Self::Local(path) => write!(f, "{}", path.display()),
        }
    }
}

/// A component of a repository name or revision: letters, digits and
/// `-`, `_`, `.`, without `..`, so that it is safe in a URL and a path.
fn is_safe_component(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 96
        && !text.starts_with('.')
        && !text.contains("..")
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

impl ModelSpec {
    /// Parses a model string.
    ///
    /// A string is a local path when it is absolute or starts with `./`,
    /// `../`, `~` or `file://`; otherwise `org/name[@revision]` names a
    /// Hugging Face repository. `org/name` is never read as a relative path.
    ///
    /// # Errors
    ///
    /// Returns `model.invalid_spec` when the string is neither form.
    pub fn parse(text: &str) -> Result<Self> {
        if text == "default" {
            return Ok(Self::Default);
        }
        if let Some(path) = text.strip_prefix("file://") {
            return Ok(Self::Local(PathBuf::from(path)));
        }
        if let Some(rest) = text.strip_prefix('~') {
            let home = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .ok_or_else(|| Error::invalid_model_spec(text, "the home directory is unknown"))?;
            let relative = rest.trim_start_matches(['/', '\\']);
            return Ok(Self::Local(PathBuf::from(home).join(relative)));
        }
        let path = PathBuf::from(text);
        if path.is_absolute() || text.starts_with("./") || text.starts_with("../") {
            return Ok(Self::Local(path));
        }

        let (repository, revision) = match text.split_once('@') {
            Some((repository, revision)) => (repository, Some(revision)),
            None => (text, None),
        };
        let Some((organization, name)) = repository.split_once('/') else {
            return Err(Error::invalid_model_spec(
                text,
                "expected \"default\", \"org/name[@revision]\" or a local path",
            ));
        };
        if !is_safe_component(organization) || !is_safe_component(name) {
            return Err(Error::invalid_model_spec(
                text,
                "repository names use letters, digits, '-', '_' and '.'",
            ));
        }
        if revision.is_some_and(|revision| !is_safe_component(revision)) {
            return Err(Error::invalid_model_spec(
                text,
                "revisions use letters, digits, '-', '_' and '.'",
            ));
        }
        Ok(Self::Hub {
            repository: Repository {
                organization: organization.to_owned(),
                name: name.to_owned(),
            },
            revision: revision.map(str::to_owned),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub(organization: &str, name: &str, revision: Option<&str>) -> ModelSpec {
        ModelSpec::Hub {
            repository: Repository {
                organization: organization.into(),
                name: name.into(),
            },
            revision: revision.map(Into::into),
        }
    }

    #[test]
    fn parses_every_form() {
        assert_eq!(ModelSpec::parse("default").unwrap(), ModelSpec::Default);
        assert_eq!(
            ModelSpec::parse("minishlab/potion-base-8M").unwrap(),
            hub("minishlab", "potion-base-8M", None)
        );
        assert_eq!(
            ModelSpec::parse("minishlab/potion-base-8M@bf8b0566").unwrap(),
            hub("minishlab", "potion-base-8M", Some("bf8b0566"))
        );
        assert_eq!(
            ModelSpec::parse("/opt/models/m").unwrap(),
            ModelSpec::Local("/opt/models/m".into())
        );
        assert_eq!(
            ModelSpec::parse("./models/m").unwrap(),
            ModelSpec::Local("./models/m".into())
        );
        assert_eq!(
            ModelSpec::parse("file:///opt/m").unwrap(),
            ModelSpec::Local("/opt/m".into())
        );
    }

    #[test]
    fn repository_names_are_never_relative_paths() {
        assert!(matches!(
            ModelSpec::parse("models/m").unwrap(),
            ModelSpec::Hub { .. }
        ));
    }

    #[test]
    fn rejects_malformed_and_unsafe_references() {
        for text in [
            "",
            "potion",
            "org/",
            "/name".trim_start_matches('/'),
            "org/na me",
            "org/..",
            "org/name@",
            "org/name@../main",
            "a/b/c",
        ] {
            let error = ModelSpec::parse(text).unwrap_err();
            assert_eq!(
                error.code(),
                univo_core::ErrorCode::InvalidModelSpec,
                "{text:?}"
            );
        }
    }

    #[test]
    fn displays_the_canonical_form() {
        assert_eq!(hub("org", "name", Some("v1")).to_string(), "org/name@v1");
        assert_eq!(ModelSpec::Default.to_string(), "default");
    }
}
