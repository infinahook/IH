//! Helpers internes : erreurs, identifiants, JSON, états et motifs `LIKE`.

use ih_core::events::{AgentState, TaskState};
use ih_core::{IhError, IhResult, MessageId, ProjectId, SessionId, TaskId, ThreadId};
use rusqlite::types::Type;
use rusqlite::Row;
use serde::de::DeserializeOwned;
use serde::Serialize;

/// Convertit une erreur SQLite en [`IhError::Db`] en conservant le contexte
/// de l'opération (table, intention) pour faciliter le diagnostic.
pub(crate) fn db_err(context: impl Into<String>, err: rusqlite::Error) -> IhError {
    IhError::Db(format!("{} : {err}", context.into()))
}

/// Erreur de conversion d'une colonne SQLite en type métier.
pub(crate) fn conversion_err(index: usize, err: IhError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(err))
}

/// Identifiant typé reconstruit depuis une colonne texte.
pub(crate) trait DbId: Sized {
    fn parse_db(raw: String) -> IhResult<Self>;
}

macro_rules! impl_db_id {
    ($($ty:ty),* $(,)?) => {
        $(
            impl DbId for $ty {
                fn parse_db(raw: String) -> IhResult<Self> {
                    Self::parse(raw)
                }
            }
        )*
    };
}

impl_db_id!(ProjectId, ThreadId, SessionId, TaskId, MessageId);

/// Lit un identifiant obligatoire et valide sa forme.
pub(crate) fn get_id<T: DbId>(row: &Row<'_>, index: usize) -> rusqlite::Result<T> {
    let raw: String = row.get(index)?;
    T::parse_db(raw).map_err(|err| conversion_err(index, err))
}

/// Lit un identifiant optionnel et valide sa forme.
pub(crate) fn get_opt_id<T: DbId>(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<T>> {
    match row.get::<_, Option<String>>(index)? {
        Some(raw) => T::parse_db(raw)
            .map(Some)
            .map_err(|err| conversion_err(index, err)),
        None => Ok(None),
    }
}

/// Sérialise une valeur JSON pour une colonne texte.
pub(crate) fn encode_json<T: Serialize + ?Sized>(context: &str, value: &T) -> IhResult<String> {
    serde_json::to_string(value)
        .map_err(|err| IhError::Internal(format!("{context} : sérialisation JSON : {err}")))
}

/// Lit une colonne JSON obligatoire.
pub(crate) fn decode_json<T: DeserializeOwned>(
    row: &Row<'_>,
    index: usize,
    context: &str,
) -> rusqlite::Result<T> {
    let raw: String = row.get(index)?;
    serde_json::from_str(&raw).map_err(|err| {
        conversion_err(
            index,
            IhError::Db(format!("{context} : JSON invalide : {err}")),
        )
    })
}

/// Lit une colonne JSON optionnelle.
pub(crate) fn decode_opt_json<T: DeserializeOwned>(
    row: &Row<'_>,
    index: usize,
    context: &str,
) -> rusqlite::Result<Option<T>> {
    match row.get::<_, Option<String>>(index)? {
        Some(raw) => serde_json::from_str(&raw).map(Some).map_err(|err| {
            conversion_err(
                index,
                IhError::Db(format!("{context} : JSON invalide : {err}")),
            )
        }),
        None => Ok(None),
    }
}

/// Lit un entier non signé borné (intervalle d'un workflow, par exemple).
pub(crate) fn get_u32(row: &Row<'_>, index: usize, context: &str) -> rusqlite::Result<u32> {
    let raw: i64 = row.get(index)?;
    u32::try_from(raw)
        .map_err(|_| conversion_err(index, IhError::Db(format!("{context} hors bornes : {raw}"))))
}

/// Représentation texte stable d'un état d'agent (identique à sa sérialisation
/// `snake_case` dans les événements du noyau).
pub(crate) fn agent_state_str(state: AgentState) -> &'static str {
    match state {
        AgentState::Starting => "starting",
        AgentState::Ready => "ready",
        AgentState::Running => "running",
        AgentState::WaitingApproval => "waiting_approval",
        AgentState::Completed => "completed",
        AgentState::Failed => "failed",
        AgentState::Cancelled => "cancelled",
    }
}

/// Représentation texte stable d'un état de tâche.
pub(crate) fn task_state_str(state: TaskState) -> &'static str {
    match state {
        TaskState::Pending => "pending",
        TaskState::Ready => "ready",
        TaskState::Running => "running",
        TaskState::Blocked => "blocked",
        TaskState::Completed => "completed",
        TaskState::Failed => "failed",
        TaskState::Cancelled => "cancelled",
    }
}

fn parse_agent_state(raw: &str) -> Option<AgentState> {
    Some(match raw {
        "starting" => AgentState::Starting,
        "ready" => AgentState::Ready,
        "running" => AgentState::Running,
        "waiting_approval" => AgentState::WaitingApproval,
        "completed" => AgentState::Completed,
        "failed" => AgentState::Failed,
        "cancelled" => AgentState::Cancelled,
        _ => return None,
    })
}

fn parse_task_state(raw: &str) -> Option<TaskState> {
    Some(match raw {
        "pending" => TaskState::Pending,
        "ready" => TaskState::Ready,
        "running" => TaskState::Running,
        "blocked" => TaskState::Blocked,
        "completed" => TaskState::Completed,
        "failed" => TaskState::Failed,
        "cancelled" => TaskState::Cancelled,
        _ => return None,
    })
}

/// Lit l'état d'une session ; une valeur inconnue signale une base corrompue.
pub(crate) fn get_agent_state(row: &Row<'_>, index: usize) -> rusqlite::Result<AgentState> {
    let raw: String = row.get(index)?;
    parse_agent_state(&raw).ok_or_else(|| {
        conversion_err(
            index,
            IhError::Db(format!("état d'agent inconnu en base : {raw}")),
        )
    })
}

/// Lit l'état d'une tâche ; une valeur inconnue signale une base corrompue.
pub(crate) fn get_task_state(row: &Row<'_>, index: usize) -> rusqlite::Result<TaskState> {
    let raw: String = row.get(index)?;
    parse_task_state(&raw).ok_or_else(|| {
        conversion_err(
            index,
            IhError::Db(format!("état de tâche inconnu en base : {raw}")),
        )
    })
}

/// Vérifie qu'une valeur appartient à une liste fermée.
pub(crate) fn validate_choice(field: &str, value: &str, allowed: &[&str]) -> IhResult<()> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(IhError::Invalid(format!(
            "{field} inconnu : {value} (valeurs acceptées : {})",
            allowed.join(", ")
        )))
    }
}

/// Refuse une chaîne vide ou composée uniquement d'espaces.
pub(crate) fn require_non_empty(field: &str, value: &str) -> IhResult<()> {
    if value.trim().is_empty() {
        Err(IhError::Invalid(format!("{field} vide")))
    } else {
        Ok(())
    }
}

/// Construit un motif `LIKE` littéral : les jokers `%`, `_` et l'échappement
/// saisis par l'utilisateur sont neutralisés, puis le motif est passé en
/// paramètre lié. Aucune entrée utilisateur n'est concaténée au SQL.
pub(crate) fn like_pattern(query: &str) -> String {
    let mut pattern = String::with_capacity(query.len() + 2);
    pattern.push('%');
    for ch in query.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    pattern.push('%');
    pattern
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motif_like_echappe_les_jokers_utilisateur() {
        assert_eq!(like_pattern("100%_ok\\"), "%100\\%\\_ok\\\\%");
        assert_eq!(like_pattern("rust"), "%rust%");
    }
}
