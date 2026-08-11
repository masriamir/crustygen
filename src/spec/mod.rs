//! The map-spec document: parsing a filled template into a typed spec.
//!
//! A spec file is YAML frontmatter between `---` fences followed by a
//! Markdown body. This stage reads the spec; it does not generate an IR
//! from it and it does not compare it against a built map. See
//! `docs/map-spec.md` for the format contract and the enforcement split.

use thiserror::Error;

use crate::tables::Tables;

/// The prose body: free-text sections, an ordered sequence of events, and
/// structured secrets, hand-parsed from the Markdown [`split_frontmatter`]
/// splits off.
pub mod body;

/// Typed frontmatter groups: the fields of a filled map-spec template,
/// deserialized from the YAML [`split_frontmatter`] splits off the body.
pub mod frontmatter;

/// Post-deserialize validation: rules a filled template must satisfy that
/// serde alone cannot express (range coherence, vocabulary resolution,
/// engine-domain bounds). The module holds both the always-error set
/// [`validate::always_errors`] checks and the enforcement-governed set that
/// [`validate::run`] combines with them under the enforcement mode.
pub mod validate;

/// A parsed map-spec document: typed frontmatter plus the parsed prose body.
///
/// Produced by [`Spec::from_markdown`], which chains this module's stages —
/// [`split_frontmatter`], [`frontmatter::parse`], and [`body::parse`] — and
/// runs [`validate::run`] against the result.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    /// The document's typed YAML frontmatter.
    pub frontmatter: frontmatter::Frontmatter,
    /// The document's parsed prose body.
    pub body: body::Body,
}

impl Spec {
    /// Parses a complete map-spec document — `---`-fenced YAML frontmatter
    /// followed by a Markdown body — and validates it against `tables`.
    ///
    /// Runs, in order: [`split_frontmatter`], [`frontmatter::parse`],
    /// [`body::parse`], then [`validate::run`]. Under
    /// `constraints.enforcement: target`, [`validate::run`]'s
    /// enforcement-governed findings come back as [`SpecDocument::sacrifices`]
    /// rather than rejecting; under `strict`, the same findings surface as
    /// part of the `Err` this returns instead, so `sacrifices` is empty by
    /// construction on every `Ok` produced under `strict`.
    ///
    /// # Errors
    ///
    /// Returns [`SpecError::MissingFrontmatter`] or
    /// [`SpecError::UnterminatedFrontmatter`] if `text` is not properly
    /// fenced, [`SpecError::Frontmatter`] if the YAML frontmatter does not
    /// deserialize, one of the body-grammar variants if the Markdown body is
    /// malformed, or [`SpecError::Invalid`] if [`validate::run`] finds a
    /// violation.
    pub fn from_markdown(text: &str, tables: &Tables) -> Result<SpecDocument, SpecError> {
        let (yaml, body_text) = split_frontmatter(text)?;
        let frontmatter = frontmatter::parse(&yaml)?;
        let body = body::parse(&body_text)?;
        let sacrifices = validate::run(&frontmatter, &body, tables)?;
        Ok(SpecDocument {
            spec: Self { frontmatter, body },
            sacrifices,
        })
    }
}

/// The result of [`Spec::from_markdown`]: a parsed [`Spec`] plus every
/// enforcement-governed finding [`validate::governed`] recorded against it.
#[derive(Debug, Clone, PartialEq)]
pub struct SpecDocument {
    /// The parsed spec: typed frontmatter plus the parsed prose body.
    pub spec: Spec,
    /// Enforcement-governed findings recorded rather than rejected, per
    /// `docs/map-spec.md`'s "Errors and the enforcement split". Empty by
    /// construction under `constraints.enforcement: strict` — a nonempty
    /// finding there becomes part of [`Spec::from_markdown`]'s `Err`
    /// instead.
    pub sacrifices: Vec<Sacrifice>,
}

/// One violation [`validate::always_errors`] found: the field path at fault
/// and a human-readable message. `Display` renders `` `{path}`: {message} ``,
/// matching [`SpecError::Frontmatter`]'s path-first convention.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("`{path}`: {message}")]
pub struct Violation {
    /// The field path where the violation was found, e.g. `scale.rooms`.
    pub path: String,
    /// A human-readable description of what is wrong.
    pub message: String,
}

/// One finding from [`validate::governed`]: a place where the document's
/// declared value and its actual content disagree, recorded rather than
/// rejected under `constraints.enforcement: target`. Carries the field
/// path, the target the spec asked for, the actual value the document
/// shows, and a message phrased as which parameter was sacrificed to hold
/// which — ready for the conformance report to render verbatim, per
/// `docs/map-spec.md`'s "Errors and the enforcement split". Under
/// `enforcement: strict` the same finding becomes a [`Violation`] instead
/// ([`validate::run`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sacrifice {
    /// The field path where the finding occurred, e.g. `secrets.count`.
    pub path: String,
    /// The target the spec asked for, e.g. `"within [96, 208]"`.
    pub target: String,
    /// The actual value the document shows, e.g. `"3 prose secret
    /// sections"`.
    pub actual: String,
    /// A human-readable message phrased as which parameter was sacrificed
    /// to hold which.
    pub message: String,
}

/// A defect in a map-spec document. Every variant names its subject —
/// a field path, a section heading, or a secret's name — per
/// `docs/design.md` §9.
#[derive(Debug, Error)]
pub enum SpecError {
    /// The document does not begin with a `---` frontmatter fence.
    #[error("the document does not begin with a `---` frontmatter fence")]
    MissingFrontmatter,
    /// The opening fence is never closed by a matching `---` line.
    #[error("the `---` frontmatter fence opened on line 1 is never closed")]
    UnterminatedFrontmatter,
    /// The frontmatter YAML failed to deserialize into [`frontmatter::Frontmatter`].
    #[error("frontmatter field `{path}`: {message}")]
    Frontmatter {
        /// The field path at which deserialization failed, e.g.
        /// `progression.doors.lock_types[1]`.
        path: String,
        /// The inner serde error message.
        message: String,
    },
    /// A body `## ` heading is not one of the four recognized section
    /// names.
    #[error(
        "body: unknown section heading `## {heading}` (allowed: Overview, Sequence of events, Secrets, Notes)"
    )]
    UnknownSection {
        /// The unrecognized heading text, as written after `## `.
        heading: String,
    },
    /// A body `## ` section heading appears more than once.
    #[error("body: section `## {heading}` appears more than once")]
    DuplicateSection {
        /// The repeated heading's name.
        heading: String,
    },
    /// A line in `## Sequence of events` is not an ordered-list item.
    #[error("body: line {line} in `## Sequence of events` is not an ordered-list item")]
    MalformedSequenceItem {
        /// The 1-based line number of the offending line.
        line: usize,
    },
    /// A `### ` secret subsection heading is not of the form `Secret N —
    /// <name>`.
    #[error("body: secret heading `### {heading}` is not of the form `Secret N — <name>`")]
    MalformedSecretHeading {
        /// The unrecognized heading text, as written after `### `.
        heading: String,
    },
    /// A secret is missing one of its required `Trigger`/`Reward`/`Hint`
    /// bullets.
    #[error("body: secret `{secret}` is missing its `{field}` bullet")]
    SecretMissingField {
        /// The secret's name.
        secret: String,
        /// The missing bullet's key: `"Trigger"`, `"Reward"`, or `"Hint"`.
        field: &'static str,
    },
    /// A secret's bullet value is empty after trimming.
    #[error("body: secret `{secret}` has an empty `{field}`")]
    SecretEmptyField {
        /// The secret's name.
        secret: String,
        /// The empty bullet's key: `"Trigger"`, `"Reward"`, or `"Hint"`.
        field: &'static str,
    },
    /// A secret's `Trigger` bullet names a value outside the fixed set of
    /// trigger kinds.
    #[error(
        "body: secret `{secret}` names unknown trigger `{value}` (allowed: misaligned_texture, shootable, walkover, lift, hidden_switch)"
    )]
    UnknownSecretTrigger {
        /// The secret's name.
        secret: String,
        /// The unrecognized trigger value.
        value: String,
    },
    /// A line inside a secret is not a recognized `Trigger`/`Reward`/`Hint`
    /// bullet.
    #[error("body: secret `{secret}` has an unexpected `{bullet}` bullet")]
    UnknownSecretBullet {
        /// The secret's name.
        secret: String,
        /// The offending line's content.
        bullet: String,
    },
    /// Body content appears before the first `## ` heading.
    #[error("body: content on line {line} belongs to no `##` section")]
    ContentOutsideSections {
        /// The 1-based line number of the offending line.
        line: usize,
    },
    /// The document fails one or more of [`validate::always_errors`]'s
    /// rules. `Display` renders one violation per line, in the order
    /// `always_errors` found them — thiserror interpolates a computed
    /// expression rather than a bare field, the same join-in-`#[error]`
    /// idiom `CompileError::Playability` already uses in `compile/mod.rs`,
    /// since thiserror cannot join a `Vec` field on its own.
    #[error(
        "{}",
        .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n")
    )]
    Invalid(Vec<Violation>),
}

/// Splits a spec document at its `---` fences into `(yaml, body)`.
///
/// Line endings are normalized: a trailing `\r` on any line is dropped, so
/// a CRLF checkout or a Windows-authored spec parses identically to an LF
/// one. Both returned strings use `\n` endings.
///
/// # Errors
///
/// Returns `SpecError::MissingFrontmatter` if the document does not begin
/// with a `---` fence, or `SpecError::UnterminatedFrontmatter` if the opening
/// fence is never closed.
pub fn split_frontmatter(text: &str) -> Result<(String, String), SpecError> {
    let mut lines = text.lines(); // `lines()` already strips `\r\n` and `\n`
    if lines.next() != Some("---") {
        return Err(SpecError::MissingFrontmatter);
    }
    let mut yaml = String::new();
    for line in lines.by_ref() {
        if line == "---" {
            let mut body = String::new();
            for rest in lines {
                body.push_str(rest);
                body.push('\n');
            }
            // `lines()` drops a final newline's emptiness; an empty body
            // stays empty rather than becoming "\n".
            return Ok((yaml, body));
        }
        yaml.push_str(line);
        yaml.push('\n');
    }
    Err(SpecError::UnterminatedFrontmatter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_with_frontmatter_and_body_splits_at_the_fences() {
        let (yaml, body) = split_frontmatter("---\na: 1\n---\nbody text\n").unwrap();
        assert_eq!(yaml, "a: 1\n");
        assert_eq!(body, "body text\n");
    }

    #[test]
    fn crlf_line_endings_do_not_break_the_fence_split() {
        let (yaml, body) = split_frontmatter("---\r\na: 1\r\n---\r\nbody\r\n").unwrap();
        assert_eq!(yaml, "a: 1\n");
        assert_eq!(body, "body\n");
    }

    #[test]
    fn a_document_not_opening_with_a_fence_is_rejected() {
        assert!(matches!(
            split_frontmatter("a: 1\n---\n"),
            Err(SpecError::MissingFrontmatter)
        ));
    }

    #[test]
    fn an_unclosed_fence_is_rejected() {
        assert!(matches!(
            split_frontmatter("---\na: 1\n"),
            Err(SpecError::UnterminatedFrontmatter)
        ));
    }

    #[test]
    fn an_empty_body_is_allowed_by_the_split() {
        let (_, body) = split_frontmatter("---\na: 1\n---\n").unwrap();
        assert_eq!(body, "");
    }
}
