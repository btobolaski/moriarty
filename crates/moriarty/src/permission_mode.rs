use std::{collections::BTreeSet, fmt, str::FromStr};

use serde::{Deserialize, Serialize};

/// Exact, case-sensitive non-blank names, without trimming or aliases. Claude Code's Manual
/// mode arrives as `default`; the literal name `manual` is a distinct custom mode.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum PermissionMode {
    Default,
    Plan,
    AcceptEdits,
    Auto,
    DontAsk,
    BypassPermissions,
    Custom(CustomModeName),
}

/// Private construction keeps blank names and built-in spellings out of the custom variant.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CustomModeName(String);

impl TryFrom<String> for PermissionMode {
    type Error = &'static str;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        Ok(match name.as_str() {
            "default" => Self::Default,
            "plan" => Self::Plan,
            "acceptEdits" => Self::AcceptEdits,
            "auto" => Self::Auto,
            "dontAsk" => Self::DontAsk,
            "bypassPermissions" => Self::BypassPermissions,
            _ if name.trim().is_empty() => {
                return Err("permission mode must contain at least one non-whitespace character");
            }
            _ => Self::Custom(CustomModeName(name)),
        })
    }
}

impl FromStr for PermissionMode {
    type Err = &'static str;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::try_from(name.to_owned())
    }
}

impl From<PermissionMode> for String {
    fn from(mode: PermissionMode) -> Self {
        mode.to_string()
    }
}

impl fmt::Display for PermissionMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Default => "default",
            Self::Plan => "plan",
            Self::AcceptEdits => "acceptEdits",
            Self::Auto => "auto",
            Self::DontAsk => "dontAsk",
            Self::BypassPermissions => "bypassPermissions",
            Self::Custom(name) => &name.0,
        })
    }
}

/// Rules without a restriction are permanently unrestricted, while a mode-less replay or test
/// evaluation cannot satisfy a restricted rule.
pub fn is_mode_eligible(
    restriction: Option<&BTreeSet<PermissionMode>>,
    current: Option<&PermissionMode>,
) -> bool {
    match restriction {
        None => true,
        Some(modes) => current.is_some_and(|mode| modes.contains(mode)),
    }
}

/// Shadowing is possible only when both rules can participate in at least one common mode.
pub fn mode_restrictions_overlap(
    left: Option<&BTreeSet<PermissionMode>>,
    right: Option<&BTreeSet<PermissionMode>>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (None, Some(modes)) | (Some(modes), None) => !modes.is_empty(),
        (Some(left), Some(right)) => left.iter().any(|mode| right.contains(mode)),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Debug, Parser)]
    struct ModeArgs {
        #[arg(long)]
        mode: PermissionMode,
    }

    #[test]
    fn all_modes_round_trip_with_exact_wire_and_cli_spellings() {
        for (spelling, mode) in [
            ("default", PermissionMode::Default),
            ("plan", PermissionMode::Plan),
            ("acceptEdits", PermissionMode::AcceptEdits),
            ("auto", PermissionMode::Auto),
            ("dontAsk", PermissionMode::DontAsk),
            ("bypassPermissions", PermissionMode::BypassPermissions),
            ("review-only", "review-only".parse().unwrap()),
            ("manual", "manual".parse().unwrap()),
            ("Review", "Review".parse().unwrap()),
            (" review ", " review ".parse().unwrap()),
        ] {
            assert_eq!(mode.to_string(), spelling);
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(json, format!(r#""{spelling}""#));
            assert_eq!(serde_json::from_str::<PermissionMode>(&json).unwrap(), mode);
            assert_eq!(
                ModeArgs::try_parse_from(["test", "--mode", spelling])
                    .unwrap()
                    .mode,
                mode
            );
        }
        assert_ne!(
            "manual".parse::<PermissionMode>().unwrap(),
            PermissionMode::Default
        );
        let distinct: BTreeSet<PermissionMode> = ["review", "Review", " review "]
            .map(|name| name.parse().unwrap())
            .into();
        assert_eq!(distinct.len(), 3);
    }

    #[test]
    fn blank_names_and_non_string_serde_values_are_rejected() {
        assert!("\u{2003}".parse::<PermissionMode>().is_err());
        assert!(serde_json::from_str::<PermissionMode>("1").is_err());
        assert!(ModeArgs::try_parse_from(["test", "--mode", " "]).is_err());
    }

    #[test]
    fn eligibility_and_overlap_handle_unrestricted_empty_and_restricted_sets() {
        let empty = BTreeSet::new();
        let review = "review".parse().unwrap();
        let other: PermissionMode = "Review".parse().unwrap();
        let restricted = BTreeSet::from([review]);
        let disjoint = BTreeSet::from([other.clone()]);
        let current = restricted.first();

        assert!(is_mode_eligible(None, None));
        assert!(is_mode_eligible(None, current));
        assert!(!is_mode_eligible(Some(&empty), current));
        assert!(!is_mode_eligible(Some(&restricted), None));
        assert!(is_mode_eligible(Some(&restricted), current));
        assert!(!is_mode_eligible(Some(&restricted), Some(&other)));
        assert!(mode_restrictions_overlap(None, None));
        assert!(mode_restrictions_overlap(None, Some(&restricted)));
        assert!(!mode_restrictions_overlap(None, Some(&empty)));
        assert!(mode_restrictions_overlap(
            Some(&restricted),
            Some(&restricted)
        ));
        assert!(!mode_restrictions_overlap(
            Some(&restricted),
            Some(&disjoint)
        ));
    }
}
