//! Graphe de tâches : dépendances, détection de cycle, disponibilité.

use ih_core::events::TaskState;
use ih_core::{IhError, IhResult, TaskId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamRole {
    Lead,
    Worker,
    Reviewer,
}

impl TeamRole {
    pub fn parse(value: &str) -> IhResult<Self> {
        match value {
            "lead" => Ok(Self::Lead),
            "worker" => Ok(Self::Worker),
            "reviewer" => Ok(Self::Reviewer),
            other => Err(IhError::Invalid(format!("rôle inconnu : {other}"))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lead => "lead",
            Self::Worker => "worker",
            Self::Reviewer => "reviewer",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskNode {
    pub id: TaskId,
    pub title: String,
    pub description: String,
    pub role: TeamRole,
    pub assignee: Option<String>,
    pub depends_on: Vec<TaskId>,
    pub state: TaskState,
}

impl TaskNode {
    pub fn new(title: impl Into<String>, description: impl Into<String>, role: TeamRole) -> Self {
        Self {
            id: TaskId::new(),
            title: title.into(),
            description: description.into(),
            role,
            assignee: None,
            depends_on: Vec::new(),
            state: TaskState::Pending,
        }
    }

    pub fn with_assignee(mut self, agent: impl Into<String>) -> Self {
        self.assignee = Some(agent.into());
        self
    }

    pub fn after(mut self, dependency: &TaskId) -> Self {
        self.depends_on.push(dependency.clone());
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskGraph {
    nodes: BTreeMap<String, TaskNode>,
}

impl TaskGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, node: TaskNode) -> IhResult<TaskId> {
        if node.title.trim().is_empty() {
            return Err(IhError::Invalid("titre de tâche vide".into()));
        }
        let id = node.id.clone();
        if self.nodes.contains_key(id.as_str()) {
            return Err(IhError::Conflict(format!("tâche déjà présente : {id}")));
        }
        self.nodes.insert(id.to_string(), node);
        Ok(id)
    }

    pub fn get(&self, id: &TaskId) -> Option<&TaskNode> {
        self.nodes.get(id.as_str())
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &TaskNode> {
        self.nodes.values()
    }

    /// Ordre d'exécution : dépendances d'abord, puis Lead, Workers, Reviewer.
    ///
    /// `iter()` suit l'ordre des clés (identifiants aléatoires) : il n'est pas
    /// exécutable tel quel. Ce tri garantit qu'aucune tâche ne précède une
    /// dépendance et que la relecture reste en dernier.
    pub fn topological_order(&self) -> Vec<&TaskNode> {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        let mut remaining: BTreeMap<&str, usize> = BTreeMap::new();
        let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (id, node) in &self.nodes {
            remaining.insert(id.as_str(), node.depends_on.len());
            for dep in &node.depends_on {
                children.entry(dep.as_str()).or_default().push(id.as_str());
            }
        }
        let priority = |node: &TaskNode| match node.role {
            TeamRole::Lead => 0u8,
            TeamRole::Worker => 1,
            TeamRole::Reviewer => 2,
        };
        let mut ready: BinaryHeap<Reverse<(u8, String)>> = remaining
            .iter()
            .filter(|(_, count)| **count == 0)
            .filter_map(|(id, _)| self.nodes.get(*id))
            .map(|node| Reverse((priority(node), node.id.to_string())))
            .collect();
        let mut ordered = Vec::with_capacity(self.nodes.len());
        while let Some(Reverse((_, id))) = ready.pop() {
            if let Some(node) = self.nodes.get(&id) {
                ordered.push(node);
            }
            if let Some(dependents) = children.get(id.as_str()) {
                for child in dependents {
                    if let Some(count) = remaining.get_mut(*child) {
                        *count -= 1;
                        if *count == 0 {
                            if let Some(node) = self.nodes.get(*child) {
                                ready.push(Reverse((priority(node), node.id.to_string())));
                            }
                        }
                    }
                }
            }
        }
        ordered
    }

    /// Vérifie que chaque dépendance existe et qu'il n'existe aucun cycle.
    pub fn validate(&self) -> IhResult<()> {
        for node in self.nodes.values() {
            for dep in &node.depends_on {
                if !self.nodes.contains_key(dep.as_str()) {
                    return Err(IhError::Invalid(format!(
                        "la tâche {} dépend d'une tâche inconnue {dep}",
                        node.id
                    )));
                }
                if dep == &node.id {
                    return Err(IhError::Invalid(format!(
                        "la tâche {} dépend d'elle-même",
                        node.id
                    )));
                }
            }
        }
        // Détection de cycle par parcours en profondeur avec marquage.
        let mut visiting = BTreeSet::new();
        let mut visited = BTreeSet::new();
        for id in self.nodes.keys() {
            self.visit(id, &mut visiting, &mut visited)?;
        }
        Ok(())
    }

    fn visit(
        &self,
        id: &str,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
    ) -> IhResult<()> {
        if visited.contains(id) {
            return Ok(());
        }
        if !visiting.insert(id.to_string()) {
            return Err(IhError::Invalid(format!(
                "cycle détecté dans le graphe de tâches (nœud {id})"
            )));
        }
        if let Some(node) = self.nodes.get(id) {
            for dep in &node.depends_on {
                self.visit(dep.as_str(), visiting, visited)?;
            }
        }
        visiting.remove(id);
        visited.insert(id.to_string());
        Ok(())
    }

    /// Tâches exécutables maintenant : en attente et dont toutes les
    /// dépendances sont terminées. Résultat trié par identifiant pour rester
    /// déterministe.
    pub fn ready_tasks(&self) -> Vec<&TaskNode> {
        let mut ready: Vec<&TaskNode> = self
            .nodes
            .values()
            .filter(|node| node.state == TaskState::Pending)
            .filter(|node| {
                node.depends_on.iter().all(|dep| {
                    self.nodes
                        .get(dep.as_str())
                        .map(|d| d.state == TaskState::Completed)
                        .unwrap_or(false)
                })
            })
            .collect();
        ready.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        ready
    }

    /// Tâches bloquées parce qu'une dépendance a échoué ou été annulée.
    pub fn blocked_tasks(&self) -> Vec<&TaskNode> {
        self.nodes
            .values()
            .filter(|node| node.state == TaskState::Pending)
            .filter(|node| {
                node.depends_on.iter().any(|dep| {
                    matches!(
                        self.nodes.get(dep.as_str()).map(|d| d.state),
                        Some(TaskState::Failed) | Some(TaskState::Cancelled)
                    )
                })
            })
            .collect()
    }

    /// Applique une transition d'état à une tâche.
    pub fn set_state(&mut self, id: &TaskId, state: TaskState) -> IhResult<()> {
        let node = self
            .nodes
            .get_mut(id.as_str())
            .ok_or_else(|| IhError::NotFound(format!("tâche {id}")))?;
        if node.state.is_terminal() {
            return Err(IhError::Conflict(format!(
                "la tâche {id} est déjà terminée ({:?})",
                node.state
            )));
        }
        node.state = state;
        Ok(())
    }

    pub fn assign(&mut self, id: &TaskId, agent: &str) -> IhResult<()> {
        let node = self
            .nodes
            .get_mut(id.as_str())
            .ok_or_else(|| IhError::NotFound(format!("tâche {id}")))?;
        node.assignee = Some(agent.to_string());
        Ok(())
    }

    pub fn all_terminal(&self) -> bool {
        !self.nodes.is_empty() && self.nodes.values().all(|n| n.state.is_terminal())
    }

    pub fn completed_count(&self) -> usize {
        self.nodes
            .values()
            .filter(|n| n.state == TaskState::Completed)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphe_lineaire_respecte_l_ordre() {
        let mut graph = TaskGraph::new();
        let a = graph
            .add(TaskNode::new("Analyser", "lire le dépôt", TeamRole::Lead))
            .unwrap();
        let b = graph
            .add(TaskNode::new("Implémenter", "coder", TeamRole::Worker).after(&a))
            .unwrap();
        graph
            .add(TaskNode::new("Relire", "vérifier", TeamRole::Reviewer).after(&b))
            .unwrap();
        graph.validate().unwrap();

        let ready = graph.ready_tasks();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, a);

        graph.set_state(&a, TaskState::Completed).unwrap();
        let ready = graph.ready_tasks();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, b);
    }

    #[test]
    fn cycle_detecte() {
        let mut graph = TaskGraph::new();
        let a = TaskNode::new("A", "", TeamRole::Worker);
        let id_a = a.id.clone();
        let b = TaskNode::new("B", "", TeamRole::Worker).after(&id_a);
        let id_b = b.id.clone();
        graph.add(a).unwrap();
        graph.add(b).unwrap();
        // force le cycle : A dépend de B
        let node_a = graph.nodes.get_mut(id_a.as_str()).unwrap();
        node_a.depends_on.push(id_b);
        assert!(graph.validate().is_err());
    }

    #[test]
    fn dependance_inconnue_refusee() {
        let mut graph = TaskGraph::new();
        let ghost = TaskId::new();
        graph
            .add(TaskNode::new("A", "", TeamRole::Worker).after(&ghost))
            .unwrap();
        assert!(graph.validate().is_err());
    }

    #[test]
    fn autodependance_refusee() {
        let mut graph = TaskGraph::new();
        let node = TaskNode::new("A", "", TeamRole::Worker);
        let id = node.id.clone();
        graph.add(node).unwrap();
        graph
            .nodes
            .get_mut(id.as_str())
            .unwrap()
            .depends_on
            .push(id.clone());
        assert!(graph.validate().is_err());
    }

    #[test]
    fn echec_bloque_les_dependants() {
        let mut graph = TaskGraph::new();
        let a = graph.add(TaskNode::new("A", "", TeamRole::Lead)).unwrap();
        let _b = graph
            .add(TaskNode::new("B", "", TeamRole::Worker).after(&a))
            .unwrap();
        graph.set_state(&a, TaskState::Failed).unwrap();
        assert_eq!(graph.blocked_tasks().len(), 1);
        assert!(graph.ready_tasks().is_empty());
    }

    #[test]
    fn transitions_terminales_verrouillees() {
        let mut graph = TaskGraph::new();
        let a = graph.add(TaskNode::new("A", "", TeamRole::Worker)).unwrap();
        graph.set_state(&a, TaskState::Completed).unwrap();
        assert!(graph.set_state(&a, TaskState::Running).is_err());
        assert!(graph.all_terminal());
    }

    #[test]
    fn titre_vide_et_doublon_refuses() {
        let mut graph = TaskGraph::new();
        assert!(graph
            .add(TaskNode::new("  ", "", TeamRole::Worker))
            .is_err());
        let node = TaskNode::new("A", "", TeamRole::Worker);
        let id = node.id.clone();
        graph.add(node).unwrap();
        let mut dup = TaskNode::new("B", "", TeamRole::Worker);
        dup.id = id;
        assert!(graph.add(dup).is_err());
    }
}
