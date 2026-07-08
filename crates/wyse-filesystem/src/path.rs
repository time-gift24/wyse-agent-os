//! Virtual path handling for agent-visible filesystem paths.

use std::{fmt, str::FromStr};

/// Agent-visible absolute path inside a virtual filesystem.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VirtualPath(String);

impl VirtualPath {
    /// Returns the original virtual path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn segments(&self) -> impl Iterator<Item = &str> {
        self.0
            .trim_start_matches('/')
            .split('/')
            .filter(|segment| !segment.is_empty())
    }
}

impl TryFrom<&str> for VirtualPath {
    type Error = VirtualPathError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        validate(value)?;
        Ok(Self(value.to_owned()))
    }
}

impl FromStr for VirtualPath {
    type Err = VirtualPathError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value)
    }
}

impl fmt::Display for VirtualPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Error returned when parsing an invalid [`VirtualPath`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid virtual path")]
pub struct VirtualPathError;

fn validate(value: &str) -> Result<(), VirtualPathError> {
    if value.is_empty() || !value.starts_with('/') || value.contains('\\') || value.contains('\0') {
        return Err(VirtualPathError);
    }

    if value == "/" {
        return Ok(());
    }

    for segment in value.trim_start_matches('/').split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(VirtualPathError);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_root_and_virtual_absolute_paths() {
        assert_eq!(
            VirtualPath::try_from("/").expect("root is valid").as_str(),
            "/"
        );
        assert_eq!(
            VirtualPath::try_from("/src/lib.rs")
                .expect("absolute virtual path is valid")
                .as_str(),
            "/src/lib.rs"
        );
    }

    #[test]
    fn rejects_paths_that_are_not_safe_virtual_absolutes() {
        for value in [
            "",
            "src/lib.rs",
            "../secret",
            "/.",
            "/src/.",
            "/../secret",
            "/src/../secret",
            "/src//lib.rs",
            r"/src\\lib.rs",
            "C:/Users/me/file.txt",
            "/has\0nul",
        ] {
            assert!(
                VirtualPath::try_from(value).is_err(),
                "{value:?} should be rejected"
            );
        }
    }

    #[test]
    fn exposes_validated_segments_without_root_marker() {
        let path = VirtualPath::try_from("/src/lib.rs").expect("path is valid");
        let segments = path.segments().collect::<Vec<_>>();
        assert_eq!(segments, ["src", "lib.rs"]);
    }
}
