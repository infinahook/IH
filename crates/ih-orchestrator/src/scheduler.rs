//! Ordonnanceur : décide quelle tâche prête lancer, sans dépasser la limite
//! de parallélisme de l'équipe.

use crate::graph::{TaskGraph, TaskNode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scheduler {
    max_parallel: u8,
}

impl Scheduler {
    pub fn new(max_parallel: u8) -> Self {
        Self {
            max_parallel: max_parallel.clamp(1, 4),
        }
    }

    pub fn max_parallel(&self) -> u8 {
        self.max_parallel
    }

    pub fn can_start(&self, running: usize) -> bool {
        running < self.max_parallel as usize
    }

    /// Choisit la prochaine tâche à lancer : priorité au coordinateur, puis
    /// aux exécutants, puis aux relecteurs ; ordre déterministe ensuite.
    pub fn pick<'a>(&self, graph: &'a TaskGraph, running: usize) -> Option<&'a TaskNode> {
        if !self.can_start(running) {
            return None;
        }
        let mut ready = graph.ready_tasks();
        ready.sort_by_key(|node| match node.role {
            crate::graph::TeamRole::Lead => 0,
            crate::graph::TeamRole::Worker => 1,
            crate::graph::TeamRole::Reviewer => 2,
        });
        ready.into_iter().next()
    }

    /// Lance autant de tâches que la limite le permet et renvoie les
    /// identifiants retenus (dans l'ordre de lancement).
    pub fn dispatch(&self, graph: &TaskGraph, running: usize) -> Vec<String> {
        let mut available = self.max_parallel as usize - running.min(self.max_parallel as usize);
        if available == 0 {
            return Vec::new();
        }
        let mut ready = graph.ready_tasks();
        ready.sort_by_key(|node| match node.role {
            crate::graph::TeamRole::Lead => 0,
            crate::graph::TeamRole::Worker => 1,
            crate::graph::TeamRole::Reviewer => 2,
        });
        let mut picked = Vec::new();
        for node in ready {
            if available == 0 {
                break;
            }
            // Une seule tâche de coordination ou de relecture à la fois.
            if matches!(
                node.role,
                crate::graph::TeamRole::Lead | crate::graph::TeamRole::Reviewer
            ) && !picked.is_empty()
            {
                continue;
            }
            picked.push(node.id.to_string());
            available -= 1;
        }
        picked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::TeamRole;

    fn graph_with_workers(n: usize) -> TaskGraph {
        let mut graph = TaskGraph::new();
        for i in 0..n {
            graph
                .add(TaskNode::new(format!("W{i}"), "", TeamRole::Worker))
                .unwrap();
        }
        graph
    }

    #[test]
    fn limite_de_parallelisme() {
        let s = Scheduler::new(2);
        assert!(s.can_start(1));
        assert!(!s.can_start(2));
        assert_eq!(s.pick(&graph_with_workers(3), 2), None);
    }

    #[test]
    fn priorite_au_coordinateur() {
        let mut graph = graph_with_workers(2);
        graph
            .add(TaskNode::new("Lead", "", TeamRole::Lead))
            .unwrap();
        let s = Scheduler::new(2);
        let picked = s.pick(&graph, 0).unwrap();
        assert_eq!(picked.role, TeamRole::Lead);
    }

    #[test]
    fn dispatch_respecte_la_limite() {
        let s = Scheduler::new(2);
        let graph = graph_with_workers(5);
        assert_eq!(s.dispatch(&graph, 0).len(), 2);
        assert_eq!(s.dispatch(&graph, 1).len(), 1);
        assert!(s.dispatch(&graph, 2).is_empty());
    }

    #[test]
    fn dispatch_evite_deux_coordinateurs() {
        let mut graph = TaskGraph::new();
        graph.add(TaskNode::new("L1", "", TeamRole::Lead)).unwrap();
        graph.add(TaskNode::new("L2", "", TeamRole::Lead)).unwrap();
        let s = Scheduler::new(4);
        let picked = s.dispatch(&graph, 0);
        assert_eq!(
            picked.len(),
            1,
            "une seule coordination à la fois: {picked:?}"
        );
    }

    #[test]
    fn graphe_vide_ne_lance_rien() {
        let s = Scheduler::new(2);
        assert!(s.pick(&TaskGraph::new(), 0).is_none());
        assert!(s.dispatch(&TaskGraph::new(), 0).is_empty());
    }
}
