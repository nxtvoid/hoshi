use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Whether agents may take a secret of the vault.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum VaultAccess {
    /// Agents see it, and the user approves each use.
    #[default]
    Ask,
    /// Agents take it without asking.
    Allow,
    /// Agents never see it.
    Hidden,
}

/// One value of a secret, such as the password of a login.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VaultField {
    pub id: String,
    /// What the value is, such as "Email". Empty when the secret is its only value.
    pub label: String,
    /// Typed and shown as a password is.
    pub masked: bool,
}

/// A secret the user keeps in the vault: one value, or the ones that go together, such as
/// the email and the password of a login. The values stay in the system's credential store
/// and never travel with this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VaultSecret {
    pub id: String,
    pub name: String,
    /// What it is for, such as the account it goes with. Agents read it.
    pub description: String,
    /// `None` for a secret of every project.
    pub project_id: Option<String>,
    pub access: VaultAccess,
    /// At least one.
    pub fields: Vec<VaultField>,
    pub created_at: String,
    pub updated_at: String,
    /// When an agent last took it.
    pub last_used_at: Option<String>,
}

/// A field as the user fills it in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VaultFieldInput {
    /// The field it changes; `None` adds one.
    pub id: Option<String>,
    pub label: String,
    pub masked: bool,
    /// `None` keeps the value the field already has.
    pub value: Option<String>,
}

impl VaultFieldInput {
    /// The only field of a secret that is a single value.
    pub fn single(value: String) -> Self {
        Self {
            id: None,
            label: String::new(),
            masked: true,
            value: Some(value),
        }
    }
}

/// What the user fills in to add a secret, or to change one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VaultSecretInput {
    pub name: String,
    pub description: String,
    pub project_id: Option<String>,
    pub access: VaultAccess,
    /// The secret's fields from now on: one it had and is not here goes, with its value.
    pub fields: Vec<VaultFieldInput>,
}

/// The vault's secret a request in the transcript is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VaultSecretRef {
    pub id: String,
    pub name: String,
}

/// One of the values an agent asks for in a single card because they go together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SecretField {
    pub label: String,
    /// Typed as a password is; an email or a user name is not.
    pub masked: bool,
}

/// Keeps a secret the user typed for an agent in the vault too, for the thread's project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VaultKeep {
    pub name: String,
    pub access: VaultAccess,
}

/// The user's answer to an agent's secret request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum SecretAnswer {
    /// Typed in the card: one value for each field it asked for, in their order.
    Value {
        values: Vec<String>,
        keep: Option<VaultKeep>,
    },
    /// One the vault holds: the one the agent asked for, or the user's pick. `always` lets
    /// agents take it without asking from now on.
    Vault {
        secret_id: String,
        always: bool,
    },
    /// Keeps in the vault the values the user already wrote in a message, for a card
    /// that only asks that.
    Keep {
        keep: VaultKeep,
    },
    Decline,
}

impl VaultSecret {
    /// Whether the agent of a thread in `project_id` may know of it.
    pub fn offered_to(&self, project_id: Option<&str>) -> bool {
        self.access != VaultAccess::Hidden && self.in_reach_of(project_id)
    }

    /// Whether it belongs to every project or to `project_id`.
    pub fn in_reach_of(&self, project_id: Option<&str>) -> bool {
        self.project_id.is_none() || self.project_id.as_deref() == project_id
    }

    pub fn reference(&self) -> VaultSecretRef {
        VaultSecretRef {
            id: self.id.clone(),
            name: self.name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(project_id: Option<&str>, access: VaultAccess) -> VaultSecret {
        VaultSecret {
            id: "s".into(),
            name: "Token".into(),
            description: String::new(),
            project_id: project_id.map(str::to_owned),
            access,
            fields: Vec::new(),
            created_at: "t".into(),
            updated_at: "t".into(),
            last_used_at: None,
        }
    }

    #[test]
    fn a_secret_is_offered_in_its_project_and_a_general_one_everywhere() {
        assert!(secret(Some("p"), VaultAccess::Ask).offered_to(Some("p")));
        assert!(!secret(Some("p"), VaultAccess::Ask).offered_to(Some("other")));
        assert!(!secret(Some("p"), VaultAccess::Ask).offered_to(None));
        assert!(secret(None, VaultAccess::Allow).offered_to(Some("p")));
        assert!(secret(None, VaultAccess::Allow).offered_to(None));
    }

    #[test]
    fn a_hidden_secret_is_never_offered_but_stays_in_the_users_reach() {
        let hidden = secret(None, VaultAccess::Hidden);

        assert!(!hidden.offered_to(Some("p")));
        assert!(hidden.in_reach_of(Some("p")));
    }
}
