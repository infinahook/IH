//! Équipes et planification multi-agents.
//!
//! La planification par défaut est **déterministe** : elle structure un
//! objectif en phases (analyse, exécution parallèle bornée, relecture) sans
//! inventer de contenu. Un plan peut ensuite être remplacé par un plan produit
//! par un modèle via `ih-providers` ; le graphe et le planificateur sont
//! identiques dans les deux cas.

use crate::graph::{TaskGraph, TaskNode, TeamRole};
use crate::scheduler::Scheduler;
use ih_core::events::TaskState;
use ih_core::{IhError, IhResult, TaskId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamMember {
    pub agent_id: String,
    pub role: TeamRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Team {
    pub name: String,
    pub members: Vec<TeamMember>,
    pub max_parallel: u8,
}

impl Team {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            members: Vec::new(),
            max_parallel: 2,
        }
    }

    pub fn with_member(mut self, agent_id: impl Into<String>, role: TeamRole) -> Self {
        self.members.push(TeamMember {
            agent_id: agent_id.into(),
            role,
        });
        self
    }

    pub fn validate(&self) -> IhResult<()> {
        if self.members.is_empty() {
            return Err(IhError::Invalid("équipe sans membre".into()));
        }
        if self.max_parallel == 0 || self.max_parallel > 4 {
            return Err(IhError::Config(
                "max_parallel d'équipe doit être entre 1 et 4".into(),
            ));
        }
        for member in &self.members {
            if member.agent_id.trim().is_empty() {
                return Err(IhError::Invalid("membre d'équipe sans agent".into()));
            }
        }
        // Un même CLI peut tenir plusieurs rôles : chaque rôle exécute ses
        // propres sessions. Avec un seul agent installé, l'équipe doit rester
        // utilisable (lead + workers + reviewer sur le même adaptateur).
        Ok(())
    }

    fn agents_for(&self, role: TeamRole) -> Vec<&str> {
        self.members
            .iter()
            .filter(|m| m.role == role)
            .map(|m| m.agent_id.as_str())
            .collect()
    }
}

/// Plan prêt à être exécuté : graphe + affectations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub goal: String,
    pub graph: TaskGraph,
    pub team: Team,
}

impl Plan {
    pub fn scheduler(&self) -> Scheduler {
        Scheduler::new(self.team.max_parallel)
    }

    /// Prochaine tâche à lancer, en respectant la limite de parallélisme.
    pub fn next_task(&self, running: usize) -> Option<&TaskNode> {
        if running >= self.team.max_parallel as usize {
            return None;
        }
        self.graph.ready_tasks().into_iter().next()
    }
}

/// Décompose un objectif en un plan déterministe : analyse → N exécutions →
/// relecture. `tasks` contrôle le nombre de tâches d'exécution (1 à 4).
pub fn plan_goal(goal: &str, team: &Team, tasks: usize) -> IhResult<Plan> {
    if goal.trim().is_empty() {
        return Err(IhError::Invalid("objectif vide".into()));
    }
    team.validate()?;
    let workers = tasks.clamp(1, 4);
    let mut graph = TaskGraph::new();

    let lead_agent = team
        .agents_for(TeamRole::Lead)
        .first()
        .copied()
        .unwrap_or_else(|| team.members[0].agent_id.as_str())
        .to_string();

    let analyse = TaskNode::new(
        "Analyser l'objectif",
        format!("Décomposer et clarifier : {goal}"),
        TeamRole::Lead,
    )
    .with_assignee(lead_agent);
    let analyse_id = graph.add(analyse)?;

    let worker_agents: Vec<String> = team
        .agents_for(TeamRole::Worker)
        .into_iter()
        .map(str::to_string)
        .collect();

    let mut execution_ids = Vec::new();
    for index in 0..workers {
        let agent = worker_agents
            .get(index % worker_agents.len().max(1))
            .cloned()
            .unwrap_or_else(|| team.members[0].agent_id.clone());
        let node = TaskNode::new(
            format!("Exécuter la partie {}", index + 1),
            format!("Traiter une partie de l'objectif : {goal}"),
            TeamRole::Worker,
        )
        .with_assignee(agent)
        .after(&analyse_id);
        execution_ids.push(graph.add(node)?);
    }

    let review_agent = team
        .agents_for(TeamRole::Reviewer)
        .first()
        .map(|s| s.to_string())
        .or_else(|| {
            team.agents_for(TeamRole::Lead)
                .first()
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| team.members[0].agent_id.clone());

    let mut review = TaskNode::new(
        "Relire et valider",
        format!("Vérifier la cohérence du résultat pour : {goal}"),
        TeamRole::Reviewer,
    )
    .with_assignee(review_agent);
    for id in &execution_ids {
        review = review.after(id);
    }
    graph.add(review)?;
    graph.validate()?;

    Ok(Plan {
        goal: goal.to_string(),
        graph,
        team: team.clone(),
    })
}

/// Marque une tâche comme terminée et renvoie les tâches devenues prêtes.
pub fn complete(plan: &mut Plan, id: &TaskId) -> IhResult<Vec<TaskId>> {
    plan.graph.set_state(id, TaskState::Completed)?;
    Ok(plan
        .graph
        .ready_tasks()
        .into_iter()
        .map(|t| t.id.clone())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn team() -> Team {
        Team::new("dev")
            .with_member("claude-code", TeamRole::Lead)
            .with_member("codex", TeamRole::Worker)
            .with_member("gemini-cli", TeamRole::Reviewer)
    }

    #[test]
    fn plan_valide_avec_phases_ordonnees() {
        let plan = plan_goal("ajouter un mode hors-ligne", &team(), 2).unwrap();
        assert_eq!(plan.graph.len(), 4); // analyse + 2 exécutions + relecture
        plan.graph.validate().unwrap();
        assert_eq!(plan.graph.ready_tasks().len(), 1);
        assert_eq!(plan.graph.ready_tasks()[0].role, TeamRole::Lead);
    }

    #[test]
    fn execution_attend_l_analyse_puis_relecture_attend_tout() {
        let mut plan = plan_goal("objectif", &team(), 2).unwrap();
        let analyse = plan.graph.ready_tasks()[0].id.clone();
        complete(&mut plan, &analyse).unwrap();
        let ready = plan.graph.ready_tasks();
        assert_eq!(ready.len(), 2);
        assert!(ready.iter().all(|t| t.role == TeamRole::Worker));

        for task in ready.iter().map(|t| t.id.clone()).collect::<Vec<_>>() {
            complete(&mut plan, &task).unwrap();
        }
        let ready = plan.graph.ready_tasks();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].role, TeamRole::Reviewer);
        assert!(ready[0].depends_on.len() == 2);
    }

    #[test]
    fn limite_de_parallelisme_respectee() {
        let mut plan = plan_goal("objectif", &team(), 3).unwrap();
        let analyse = plan.graph.ready_tasks()[0].id.clone();
        complete(&mut plan, &analyse).unwrap();
        assert!(plan.next_task(0).is_some());
        assert!(plan.next_task(1).is_some());
        assert!(plan.next_task(2).is_none(), "max_parallel = 2");
    }

    #[test]
    fn equipe_invalide_refusee() {
        let invalid = Team {
            name: "x".into(),
            members: vec![],
            max_parallel: 2,
        };
        assert!(invalid.validate().is_err());

        // Un seul CLI installé : le même agent peut couvrir plusieurs rôles.
        let same_agent = Team::new("x")
            .with_member("a", TeamRole::Lead)
            .with_member("a", TeamRole::Worker)
            .with_member("a", TeamRole::Reviewer);
        same_agent.validate().unwrap();
    }

    #[test]
    fn objectif_vide_refuse() {
        assert!(plan_goal("  ", &team(), 1).is_err());
    }

    #[test]
    fn royaume_des_roles_parse() {
        assert_eq!(TeamRole::parse("lead").unwrap(), TeamRole::Lead);
        assert!(TeamRole::parse("chef").is_err());
        assert_eq!(TeamRole::Reviewer.as_str(), "reviewer");
    }
}
