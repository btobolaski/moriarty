use std::{collections::BTreeSet, fmt};

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

/// Claude Code's wire-level permission mode. Manual mode arrives as `default`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ValueEnum,
)]
#[serde(rename_all = "camelCase")]
#[value(rename_all = "camelCase")]
pub enum PermissionMode {
    Default,
    Plan,
    AcceptEdits,
    Auto,
    DontAsk,
    BypassPermissions,
}

impl fmt::Display for PermissionMode {
    /// Delegating labels to clap's derived value metadata keeps logs and CLI spellings identical.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = self
            .to_possible_value()
            .expect("every PermissionMode variant has a clap value name");
        formatter.write_str(value.get_name())
    }
}

/// Rules without a restriction are permanently unrestricted, while a mode-less replay or test
/// evaluation cannot satisfy a restricted rule.
pub fn is_mode_eligible(
    restriction: Option<&BTreeSet<PermissionMode>>,
    current: Option<PermissionMode>,
) -> bool {
    match restriction {
        None => true,
        Some(modes) => current.is_some_and(|mode| modes.contains(&mode)),
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
    use clap::{Parser, error::ErrorKind};

    use super::*;

    #[derive(Debug, Parser)]
    struct ModeArgs {
        #[arg(long, value_enum)]
        mode: PermissionMode,
    }

    #[test]
    fn all_modes_round_trip_with_exact_wire_and_cli_spellings() {
        for &mode in PermissionMode::value_variants() {
            let spelling = mode.to_string();
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(json, format!(r#""{spelling}""#));
            assert_eq!(serde_json::from_str::<PermissionMode>(&json).unwrap(), mode);
            assert_eq!(
                ModeArgs::try_parse_from(["test", "--mode", spelling.as_str()])
                    .unwrap()
                    .mode,
                mode
            );
        }
    }

    #[test]
    fn manual_and_unknown_modes_are_rejected() {
        for spelling in ["manual", "unknown"] {
            assert!(serde_json::from_str::<PermissionMode>(&format!(r#""{spelling}""#)).is_err());
            assert_eq!(
                ModeArgs::try_parse_from(["test", "--mode", spelling])
                    .unwrap_err()
                    .kind(),
                ErrorKind::InvalidValue
            );
        }
    }

    #[test]
    fn eligibility_and_overlap_handle_unrestricted_empty_and_restricted_sets() {
        let empty = BTreeSet::new();
        let plan = BTreeSet::from([PermissionMode::Plan]);
        let default = BTreeSet::from([PermissionMode::Default]);

        assert!(is_mode_eligible(None, None));
        assert!(is_mode_eligible(None, Some(PermissionMode::Plan)));
        assert!(!is_mode_eligible(Some(&empty), Some(PermissionMode::Plan)));
        assert!(!is_mode_eligible(Some(&plan), None));
        assert!(is_mode_eligible(Some(&plan), Some(PermissionMode::Plan)));
        assert!(!is_mode_eligible(
            Some(&plan),
            Some(PermissionMode::Default)
        ));

        assert!(mode_restrictions_overlap(None, None));
        assert!(mode_restrictions_overlap(None, Some(&plan)));
        assert!(!mode_restrictions_overlap(None, Some(&empty)));
        assert!(mode_restrictions_overlap(Some(&plan), Some(&plan)));
        assert!(!mode_restrictions_overlap(Some(&plan), Some(&default)));
    }
}
