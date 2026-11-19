use alloc::string::{String, ToString};
use core::fmt;

/// Broad diagnostic category, containing no input values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// Invalid TOML, unknown field, or deserialization failure.
    Syntax,
    /// A resource or input-size limit was exceeded.
    Limit,
    /// A field value is invalid.
    Value,
    /// A source reference or selector is invalid.
    Reference,
    /// Widgets overlap, exceed the grid, or are too small.
    Layout,
    /// History reservations exceed device limits.
    History,
    /// Storage could not supply the configuration.
    Storage,
}

/// A one-based position in the configuration document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    /// Line number.
    pub line: usize,
    /// Unicode character column, rather than byte column.
    pub column: usize,
}

/// Secret-free diagnostic. Field paths use schema names and numeric indices only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    /// Broad diagnostic category.
    pub kind: ErrorKind,
    /// Configuration field path, never the supplied field value.
    pub field: String,
    /// Fixed explanatory text, never a raw parser error.
    pub message: &'static str,
    /// Parser position, when available.
    pub position: Option<Position>,
    /// Requested resource count and allowed limit, where relevant.
    pub capacity: Option<(usize, usize)>,
}

impl ConfigError {
    /// Categorized board I/O failure without underlying private data.
    pub fn storage_failure() -> Self {
        Self::new(
            ErrorKind::Storage,
            "storage.file",
            "MicroSD configuration read failed",
        )
    }

    pub(super) fn new(kind: ErrorKind, field: &str, message: &'static str) -> Self {
        Self {
            kind,
            field: field.to_string(),
            message,
            position: None,
            capacity: None,
        }
    }

    pub(super) fn limit(kind: ErrorKind, field: &str, requested: usize, allowed: usize) -> Self {
        let mut error = Self::new(kind, field, "resource limit exceeded");
        error.capacity = Some((requested, allowed));
        error
    }

    pub(super) fn syntax(error: toml::de::Error, document: &str) -> Self {
        let position = error.span().map(|span| {
            let mut position = Position { line: 1, column: 1 };
            for (offset, ch) in document.char_indices() {
                if offset >= span.start {
                    break;
                }
                if ch == '\n' {
                    position.line += 1;
                    position.column = 1;
                } else {
                    position.column += 1;
                }
            }
            position
        });
        let mut safe = Self::new(
            ErrorKind::Syntax,
            "document",
            "invalid TOML, unknown field, or invalid field type",
        );
        safe.position = position;
        safe
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)?;
        if let Some((requested, allowed)) = self.capacity {
            write!(f, " (requested {requested}, limit {allowed})")?;
        }
        if let Some(position) = self.position {
            write!(f, " at line {}, column {}", position.line, position.column)?;
        }
        Ok(())
    }
}

impl core::error::Error for ConfigError {}
