//! The prose body of a spec document: free-text sections, an ordered
//! sequence of events, and structured secrets.
//!
//! Hand-rolled and line-based via [`str::lines`], which already normalizes
//! CRLF (a trailing `\r` is stripped), matching
//! [`crate::spec::split_frontmatter`]'s approach — no regex crate is used or
//! added anywhere in this parser. See `docs/map-spec.md` for the body's
//! format contract.

/// The parsed prose body of a spec document: the four `##` sections defined
/// in `docs/design.md` §5. Every section is optional; an absent section
/// contributes an empty string or an empty vector.
#[derive(Debug, Clone, PartialEq)]
pub struct Body {
    /// The `## Overview` section, verbatim; empty if the section is absent.
    pub overview: String,
    /// The `## Sequence of events` section's ordered-list items, in order;
    /// empty if the section is absent.
    pub sequence_of_events: Vec<String>,
    /// The `## Secrets` section's per-secret entries, in document order;
    /// empty if the section is absent.
    pub secrets: Vec<SecretEntry>,
    /// The `## Notes` section, verbatim; empty if the section is absent.
    pub notes: String,
}

/// One `### Secret N — <name>` subsection of the `## Secrets` section.
#[derive(Debug, Clone, PartialEq)]
pub struct SecretEntry {
    /// The secret's name, taken from its subsection heading.
    pub name: String,
    /// How the secret is triggered.
    pub trigger: SecretTrigger,
    /// What the secret rewards, from its `Reward` bullet.
    pub reward: String,
    /// The in-map hint for the secret, from its `Hint` bullet.
    pub hint: String,
}

/// How a secret is triggered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretTrigger {
    /// A wall texture that does not quite line up with its neighbors.
    MisalignedTexture,
    /// Shooting a specific target.
    Shootable,
    /// Walking onto a line.
    Walkover,
    /// Riding a lift.
    Lift,
    /// A switch disguised as ordinary scenery.
    HiddenSwitch,
}

/// Parses a spec document's prose body.
///
/// # Errors
///
/// Returns a `SpecError` variant naming the specific defect: content before
/// the first `##` heading, an unknown or duplicate `##` heading, a
/// malformed `Sequence of events` item, a malformed `### Secret` heading, or
/// an incomplete, empty, or unrecognized secret bullet. See
/// `docs/map-spec.md` for the full grammar.
pub fn parse(body: &str) -> Result<Body, crate::spec::SpecError> {
    // Single pass: group lines under their `## ` heading, in order,
    // rejecting content that appears before the first heading, an unknown
    // heading name, or a repeated heading.
    let mut sections: Vec<(&'static str, Vec<(usize, &str)>)> = Vec::new();
    let mut current: Option<usize> = None;

    for (idx, line) in body.lines().enumerate() {
        let line_no = idx + 1;

        if let Some(name) = line.strip_prefix("## ") {
            let heading = match name {
                "Overview" => "Overview",
                "Sequence of events" => "Sequence of events",
                "Secrets" => "Secrets",
                "Notes" => "Notes",
                _ => {
                    return Err(crate::spec::SpecError::UnknownSection {
                        heading: name.to_string(),
                    });
                }
            };
            if sections.iter().any(|(h, _)| *h == heading) {
                return Err(crate::spec::SpecError::DuplicateSection {
                    heading: heading.to_string(),
                });
            }
            sections.push((heading, Vec::new()));
            current = Some(sections.len() - 1);
            continue;
        }

        match current {
            Some(section) => sections[section].1.push((line_no, line)),
            None if line.trim().is_empty() => {}
            None => {
                return Err(crate::spec::SpecError::ContentOutsideSections { line: line_no });
            }
        }
    }

    let mut overview = String::new();
    let mut sequence_of_events = Vec::new();
    let mut secrets = Vec::new();
    let mut notes = String::new();

    for (heading, lines) in sections {
        match heading {
            "Overview" => overview = parse_overview(&lines),
            "Sequence of events" => sequence_of_events = parse_sequence(&lines)?,
            "Secrets" => secrets = parse_secrets(&lines)?,
            "Notes" => notes = parse_overview(&lines),
            _ => unreachable!("only the four known headings are ever collected above"),
        }
    }

    Ok(Body {
        overview,
        sequence_of_events,
        secrets,
        notes,
    })
}

/// Joins a section's lines verbatim, trimmed of leading and trailing blank
/// lines. Shared by `Overview` and `Notes`, which parse identically.
fn parse_overview(lines: &[(usize, &str)]) -> String {
    let Some(start) = lines.iter().position(|(_, l)| !l.trim().is_empty()) else {
        return String::new();
    };
    let end = lines
        .iter()
        .rposition(|(_, l)| !l.trim().is_empty())
        .unwrap_or(start);
    lines[start..=end]
        .iter()
        .map(|(_, l)| *l)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parses a `## Sequence of events` section's lines into ordered-list item
/// text, in order.
///
/// Each non-blank line must match `^[0-9]+\.\s+(.*)$`, except a line that
/// starts with whitespace and does not itself start a new item, which is
/// treated as a word-wrapped continuation of the previous item's text (the
/// shipped template wraps long steps across lines indented to align under
/// the item marker); any other non-matching non-blank line is rejected.
fn parse_sequence(lines: &[(usize, &str)]) -> Result<Vec<String>, crate::spec::SpecError> {
    let mut items: Vec<String> = Vec::new();
    for (line_no, line) in lines {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(text) = parse_sequence_item(line) {
            items.push(text.to_string());
            continue;
        }
        if line.starts_with(char::is_whitespace)
            && let Some(last) = items.last_mut()
        {
            let continuation = line.trim();
            if !continuation.is_empty() {
                if !last.is_empty() {
                    last.push(' ');
                }
                last.push_str(continuation);
            }
            continue;
        }
        return Err(crate::spec::SpecError::MalformedSequenceItem { line: *line_no });
    }
    Ok(items)
}

/// Matches `^[0-9]+\.\s+(.*)$` by hand: one or more leading ASCII digits, a
/// literal `.`, at least one whitespace character, then the captured rest
/// of the line.
fn parse_sequence_item(line: &str) -> Option<&str> {
    let digits_end = line
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(line.len());
    if digits_end == 0 || line.as_bytes().get(digits_end) != Some(&b'.') {
        return None;
    }
    let after_dot = &line[digits_end + 1..];
    let ws_end = after_dot
        .find(|c: char| !c.is_whitespace())
        .unwrap_or(after_dot.len());
    if ws_end == 0 {
        return None;
    }
    Some(&after_dot[ws_end..])
}

/// Parses a `## Secrets` section's lines into secret entries, in order.
fn parse_secrets(lines: &[(usize, &str)]) -> Result<Vec<SecretEntry>, crate::spec::SpecError> {
    let mut secrets = Vec::new();
    let mut current: Option<SecretBuilder> = None;

    for (line_no, line) in lines {
        if line.trim().is_empty() {
            continue;
        }

        if let Some(heading) = line.strip_prefix("### ") {
            if let Some(builder) = current.take() {
                secrets.push(builder.finish()?);
            }
            let name = parse_secret_heading(heading).ok_or_else(|| {
                crate::spec::SpecError::MalformedSecretHeading {
                    heading: heading.to_string(),
                }
            })?;
            current = Some(SecretBuilder::new(name));
            continue;
        }

        let bullet = line.trim().to_string();
        let Some(builder) = current.as_mut() else {
            // Non-blank, non-`### ` content before any secret opens: no
            // secret exists to attribute it to, so the error names the line.
            return Err(crate::spec::SpecError::SecretContentOutsideSubsection { line: *line_no });
        };

        let key_value = bullet
            .strip_prefix("- ")
            .and_then(|rest| rest.split_once(':'));
        let Some((key, raw_value)) = key_value else {
            return Err(crate::spec::SpecError::UnknownSecretBullet {
                secret: builder.name.clone(),
                bullet,
            });
        };

        let value = strip_html_comment(raw_value).trim().to_string();
        match key {
            "Trigger" if builder.trigger.is_none() => {
                if value.is_empty() {
                    return Err(crate::spec::SpecError::SecretEmptyField {
                        secret: builder.name.clone(),
                        field: "Trigger",
                    });
                }
                let trigger = parse_trigger(&value).ok_or_else(|| {
                    crate::spec::SpecError::UnknownSecretTrigger {
                        secret: builder.name.clone(),
                        value: value.clone(),
                    }
                })?;
                builder.trigger = Some(trigger);
            }
            "Reward" if builder.reward.is_none() => {
                if value.is_empty() {
                    return Err(crate::spec::SpecError::SecretEmptyField {
                        secret: builder.name.clone(),
                        field: "Reward",
                    });
                }
                builder.reward = Some(value);
            }
            "Hint" if builder.hint.is_none() => {
                if value.is_empty() {
                    return Err(crate::spec::SpecError::SecretEmptyField {
                        secret: builder.name.clone(),
                        field: "Hint",
                    });
                }
                builder.hint = Some(value);
            }
            _ => {
                return Err(crate::spec::SpecError::UnknownSecretBullet {
                    secret: builder.name.clone(),
                    bullet,
                });
            }
        }
    }

    if let Some(builder) = current {
        secrets.push(builder.finish()?);
    }

    Ok(secrets)
}

/// Strips a trailing `<!-- ... -->` HTML comment from a bullet value before
/// trimming. The shipped template carries its allowed-values annotation
/// this way on every `Trigger` bullet (`docs/map-spec.md`: "every field
/// carrying its allowed-values comment"; "The comment is the contract").
fn strip_html_comment(value: &str) -> &str {
    value.find("<!--").map_or(value, |idx| &value[..idx])
}

/// Maps a trigger value to [`SecretTrigger`] by exact `snake_case` match.
fn parse_trigger(value: &str) -> Option<SecretTrigger> {
    match value {
        "misaligned_texture" => Some(SecretTrigger::MisalignedTexture),
        "shootable" => Some(SecretTrigger::Shootable),
        "walkover" => Some(SecretTrigger::Walkover),
        "lift" => Some(SecretTrigger::Lift),
        "hidden_switch" => Some(SecretTrigger::HiddenSwitch),
        _ => None,
    }
}

/// Parses a `### ` subsection heading's remainder against `Secret <digits>
/// — <name>` (em dash), returning the name. The digits are required but
/// otherwise unused semantically.
fn parse_secret_heading(remainder: &str) -> Option<String> {
    let after_secret = remainder.strip_prefix("Secret ")?;
    let digits_end = after_secret
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(after_secret.len());
    if digits_end == 0 {
        return None;
    }
    let name = after_secret[digits_end..].strip_prefix(" — ")?.trim();
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

/// Accumulates one secret subsection's bullets while its required fields
/// are still being read.
struct SecretBuilder {
    /// The secret's name, from its subsection heading.
    name: String,
    /// The `Trigger` bullet's value, once seen.
    trigger: Option<SecretTrigger>,
    /// The `Reward` bullet's value, once seen.
    reward: Option<String>,
    /// The `Hint` bullet's value, once seen.
    hint: Option<String>,
}

impl SecretBuilder {
    /// Starts a new builder for a secret with the given name and no bullets
    /// read yet.
    fn new(name: String) -> Self {
        Self {
            name,
            trigger: None,
            reward: None,
            hint: None,
        }
    }

    /// Finalizes the builder into a [`SecretEntry`], failing on the first
    /// missing required bullet, checked in `Trigger`, `Reward`, `Hint`
    /// order.
    fn finish(self) -> Result<SecretEntry, crate::spec::SpecError> {
        let Some(trigger) = self.trigger else {
            return Err(crate::spec::SpecError::SecretMissingField {
                secret: self.name,
                field: "Trigger",
            });
        };
        let Some(reward) = self.reward else {
            return Err(crate::spec::SpecError::SecretMissingField {
                secret: self.name,
                field: "Reward",
            });
        };
        let Some(hint) = self.hint else {
            return Err(crate::spec::SpecError::SecretMissingField {
                secret: self.name,
                field: "Hint",
            });
        };
        Ok(SecretEntry {
            name: self.name,
            trigger,
            reward,
            hint,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_template_body_parses_with_three_secrets() {
        let text = include_str!("../../map-spec.template.md");
        let (_, body_text) = crate::spec::split_frontmatter(text).unwrap();
        let body = parse(&body_text).unwrap();
        assert_eq!(body.secrets.len(), 3);
        assert!(!body.overview.is_empty());
        assert!(!body.sequence_of_events.is_empty());
        assert!(
            body.secrets
                .iter()
                .all(|s| !s.reward.is_empty() && !s.hint.is_empty())
        );
    }

    #[test]
    fn a_missing_optional_section_yields_empty_content() {
        let body = parse("## Overview\n\nJust a mood.\n").unwrap();
        assert_eq!(body.overview, "Just a mood.");
        assert!(body.sequence_of_events.is_empty());
        assert!(body.secrets.is_empty());
        assert_eq!(body.notes, "");
    }

    #[test]
    fn an_unknown_section_heading_is_rejected_by_name() {
        let err = parse("## Overveiw\n\ntext\n").unwrap_err();
        assert!(
            matches!(err, crate::spec::SpecError::UnknownSection { heading } if heading == "Overveiw")
        );
    }

    #[test]
    fn a_secret_missing_its_hint_fails_naming_the_secret() {
        let b = "## Secrets\n\n### Secret 1 — Cache\n- Trigger: walkover\n- Reward: ammo\n";
        let err = parse(b).unwrap_err();
        assert!(
            matches!(err, crate::spec::SpecError::SecretMissingField { ref secret, field: "Hint" } if secret == "Cache")
        );
    }

    #[test]
    fn content_before_any_secret_subsection_is_rejected_with_its_line() {
        let b = "## Secrets\n\n- Trigger: walkover\n";
        assert!(matches!(
            parse(b).unwrap_err(),
            crate::spec::SpecError::SecretContentOutsideSubsection { line: 3 }
        ));
    }

    #[test]
    fn a_secret_with_an_unknown_trigger_fails_naming_both() {
        let b = "## Secrets\n\n### Secret 1 — Cache\n- Trigger: humming\n- Reward: r\n- Hint: h\n";
        let err = parse(b).unwrap_err();
        assert!(
            matches!(err, crate::spec::SpecError::UnknownSecretTrigger { ref secret, ref value } if secret == "Cache" && value == "humming")
        );
    }

    #[test]
    fn a_sequence_with_a_stray_prose_line_is_rejected_with_its_line_number() {
        let b = "## Sequence of events\n\n1. Start\nthen stuff happens\n";
        assert!(matches!(
            parse(b).unwrap_err(),
            crate::spec::SpecError::MalformedSequenceItem { line: 4 }
        ));
    }

    #[test]
    fn a_duplicate_section_is_rejected() {
        let b = "## Notes\n\na\n\n## Notes\n\nb\n";
        assert!(matches!(
            parse(b).unwrap_err(),
            crate::spec::SpecError::DuplicateSection { .. }
        ));
    }

    #[test]
    fn an_empty_body_parses_to_all_empty_sections() {
        let body = parse("").unwrap();
        assert_eq!(body.overview, "");
        assert!(body.secrets.is_empty());
    }

    #[test]
    fn a_wrapped_sequence_item_reconstructs_to_the_joined_text() {
        let b = "## Sequence of events\n\n1. Start in the hub\n   and clear it\n";
        let body = parse(b).unwrap();
        assert_eq!(
            body.sequence_of_events,
            vec!["Start in the hub and clear it".to_string()]
        );
    }

    #[test]
    fn a_trigger_bullet_with_a_trailing_comment_strips_to_the_bare_value() {
        let b = "## Secrets\n\n### Secret 1 — Cache\n\
                  - Trigger: walkover   <!-- misaligned_texture | shootable | walkover | lift | hidden_switch -->\n\
                  - Reward: r\n\
                  - Hint: h\n";
        let body = parse(b).unwrap();
        assert_eq!(body.secrets[0].trigger, SecretTrigger::Walkover);
    }
}
