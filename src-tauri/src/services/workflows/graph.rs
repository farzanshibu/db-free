// SOT: workflow-graph, topological-levels, cycle-detection, repeat-expansion, run-plan, leaves-downstream

use crate::error::{AppError, AppResult};
use crate::model::workflow::{NodeConfig, SourceHandle, WorkflowEdge, WorkflowNode};
use std::collections::{HashMap, HashSet};

/// Most passes one Repeat chain may expand to (the product of nested counts).
pub const MAX_PASSES: u64 = 100_000;

// WHAT:  A workflow's nodes and edges, indexed for the engine.
// WHY:   Planning a run is graph work (ancestors, levels, cycles, loops) and
//        has nothing to do with databases, so it lives apart and is easy to test.
// HOW:   Built from the document's lists. An edge that points at a node that is
//        gone is dropped; two nodes with one id is an error.
// WHERE: src-tauri/src/services/workflows/runner.rs
pub struct Graph<'a> {
    nodes: Vec<&'a WorkflowNode>,
    by_id: HashMap<&'a str, usize>,
    edges: Vec<&'a WorkflowEdge>,
    incoming: HashMap<&'a str, Vec<&'a WorkflowEdge>>,
    outgoing: HashMap<&'a str, Vec<&'a WorkflowEdge>>,
}

// WHAT:  Nodes that run together: the same set of Repeat nodes sits upstream of
//        every target, so one run of the group's passes serves them all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub targets: Vec<String>,
    /// Every node the targets need, targets included.
    pub nodes: HashSet<String>,
    /// Repeat nodes upstream of the targets, in document order, with their counts.
    pub repeats: Vec<(String, u32)>,
}

/// Which iteration each Repeat node is on in one pass: `(i, total)`, `i` from 0.
pub type PassIndex = HashMap<String, (u32, u32)>;

impl<'a> Graph<'a> {
    pub fn new(nodes: &'a [WorkflowNode], edges: &'a [WorkflowEdge]) -> AppResult<Graph<'a>> {
        let mut by_id = HashMap::new();
        for (index, node) in nodes.iter().enumerate() {
            if by_id.insert(node.id.as_str(), index).is_some() {
                return Err(AppError::invalid_input(format!("Two nodes share the id \"{}\".", node.id)));
            }
        }
        let mut kept = Vec::new();
        let mut incoming: HashMap<&str, Vec<&WorkflowEdge>> = HashMap::new();
        let mut outgoing: HashMap<&str, Vec<&WorkflowEdge>> = HashMap::new();
        for edge in edges {
            if !by_id.contains_key(edge.source.as_str()) || !by_id.contains_key(edge.target.as_str()) {
                continue;
            }
            if edge.source == edge.target {
                return Err(AppError::invalid_input(format!("A node cannot feed itself (\"{}\").", edge.source)));
            }
            incoming.entry(edge.target.as_str()).or_default().push(edge);
            outgoing.entry(edge.source.as_str()).or_default().push(edge);
            kept.push(edge);
        }
        Ok(Graph { nodes: nodes.iter().collect(), by_id, edges: kept, incoming, outgoing })
    }

    pub fn node(&self, id: &str) -> Option<&'a WorkflowNode> {
        self.by_id.get(id).and_then(|i| self.nodes.get(*i)).copied()
    }

    pub fn nodes(&self) -> &[&'a WorkflowNode] {
        &self.nodes
    }

    pub fn edges(&self) -> &[&'a WorkflowEdge] {
        &self.edges
    }

    pub fn incoming(&self, id: &str) -> &[&'a WorkflowEdge] {
        self.incoming.get(id).map(Vec::as_slice).unwrap_or(&[])
    }

    fn outgoing(&self, id: &str) -> &[&'a WorkflowEdge] {
        self.outgoing.get(id).map(Vec::as_slice).unwrap_or(&[])
    }

    fn is_note(node: &WorkflowNode) -> bool {
        matches!(node.config, NodeConfig::Note(_))
    }

    /// Nodes nothing is wired out of, notes excepted: what "Run all" runs.
    pub fn leaves(&self) -> Vec<String> {
        self.nodes
            .iter()
            .filter(|n| !Self::is_note(n) && self.outgoing(&n.id).is_empty())
            .map(|n| n.id.clone())
            .collect()
    }

    /// The leaves reachable from a trigger (the trigger itself when it has none).
    pub fn leaves_downstream_of(&self, trigger: &str) -> Vec<String> {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut stack = vec![trigger];
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            stack.extend(self.outgoing(id).iter().map(|e| e.target.as_str()));
        }
        let leaves: Vec<String> = self
            .nodes
            .iter()
            .filter(|n| seen.contains(n.id.as_str()) && self.outgoing(&n.id).is_empty())
            .map(|n| n.id.clone())
            .collect();
        if leaves.is_empty() {
            vec![trigger.to_string()]
        } else {
            leaves
        }
    }

    /// The targets and everything upstream of them.
    pub fn ancestors(&self, targets: &[String]) -> HashSet<String> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut stack: Vec<&str> = targets.iter().map(String::as_str).collect();
        while let Some(id) = stack.pop() {
            if !seen.insert(id.to_string()) {
                continue;
            }
            stack.extend(self.incoming(id).iter().map(|e| e.source.as_str()));
        }
        seen
    }

    // WHAT:  The nodes split into levels: a node's level is after every node that
    //        feeds it, so each level can run in parallel.
    // HOW:   Kahn's algorithm over the subset; leftover nodes mean a cycle.
    pub fn levels(&self, subset: &HashSet<String>) -> AppResult<Vec<Vec<String>>> {
        let mut degree: HashMap<&str, usize> = HashMap::new();
        for node in &self.nodes {
            if subset.contains(&node.id) {
                degree.insert(node.id.as_str(), 0);
            }
        }
        for edge in &self.edges {
            if subset.contains(&edge.source) && subset.contains(&edge.target) {
                *degree.entry(edge.target.as_str()).or_insert(0) += 1;
            }
        }
        let mut levels = Vec::new();
        let mut frontier: Vec<&str> = self.nodes.iter().map(|n| n.id.as_str()).filter(|id| degree.get(id) == Some(&0)).collect();
        let mut done = 0;
        while !frontier.is_empty() {
            done += frontier.len();
            let mut next = Vec::new();
            for id in &frontier {
                for edge in self.outgoing(id) {
                    if let Some(d) = degree.get_mut(edge.target.as_str()) {
                        *d -= 1;
                        if *d == 0 {
                            next.push(edge.target.as_str());
                        }
                    }
                }
            }
            levels.push(frontier.iter().map(|id| (*id).to_string()).collect());
            frontier = next;
        }
        if done != degree.len() {
            return Err(AppError::invalid_input("The workflow has a cycle: a node feeds itself through other nodes."));
        }
        Ok(levels)
    }

    /// Would an edge from `source` to `target` close a cycle?
    pub fn would_cycle(&self, source: &str, target: &str) -> bool {
        source == target || self.ancestors(&[source.to_string()]).contains(target)
    }

    // WHAT:  Groups the targets by the Repeat nodes upstream of them.
    // WHY:   A Repeat makes everything downstream run once per iteration, so
    //        targets under the same Repeats share their passes and targets under
    //        different ones run on their own.
    pub fn plan(&self, targets: &[String]) -> AppResult<Vec<Group>> {
        let mut groups: Vec<Group> = Vec::new();
        for target in targets {
            if self.node(target).is_none() {
                return Err(AppError::not_found(format!("Node \"{target}\" does not exist.")));
            }
            let nodes = self.ancestors(std::slice::from_ref(target));
            let mut repeats = Vec::new();
            for node in &self.nodes {
                if let NodeConfig::Repeat(cfg) = &node.config {
                    if nodes.contains(&node.id) {
                        if cfg.count == 0 {
                            return Err(AppError::invalid_input(format!("Repeat \"{}\" needs a count of at least 1.", display_name(node))));
                        }
                        repeats.push((node.id.clone(), cfg.count));
                    }
                }
            }
            match groups.iter_mut().find(|g| g.repeats == repeats) {
                Some(group) => {
                    group.targets.push(target.clone());
                    group.nodes.extend(nodes);
                }
                None => groups.push(Group { targets: vec![target.clone()], nodes, repeats }),
            }
        }
        for group in &groups {
            let total: u64 = group.repeats.iter().map(|(_, count)| u64::from(*count)).try_fold(1_u64, u64::checked_mul).unwrap_or(u64::MAX);
            if total > MAX_PASSES {
                return Err(AppError::invalid_input(format!("Nested Repeat nodes would run {total} passes; the limit is {MAX_PASSES}.")));
            }
        }
        Ok(groups)
    }

    // WHAT:  The first wiring mistake among the connections between `involved` nodes, in words.
    // WHY:   A Branch has two outputs and every other node one. The canvas never lets
    //        a wire be drawn wrong, but a document written by hand or by the AI can
    //        leave a Branch connection without its side (or name a side on a node
    //        with a single output); the run would carry nothing along it and the
    //        node behind it would be skipped without a word.
    pub fn wiring_problem(&self, involved: &HashSet<&str>) -> Option<String> {
        for edge in &self.edges {
            if !involved.contains(edge.source.as_str()) || !involved.contains(edge.target.as_str()) {
                continue;
            }
            let Some(source) = self.node(&edge.source) else { continue };
            let branching = matches!(source.config, NodeConfig::Branch(_));
            match (branching, edge.source_handle) {
                (true, None) => {
                    return Some(format!("A connection from \"{}\" does not say whether it leaves by the True or the False side.", display_name(source)));
                }
                (false, Some(_)) => {
                    return Some(format!("\"{}\" has a single output, but a connection from it names a True / False side.", display_name(source)));
                }
                _ => {}
            }
        }
        None
    }

    /// The edges a node's outputs travel along, for one handle.
    pub fn edges_from(&self, id: &str, handle: Option<SourceHandle>) -> Vec<&'a WorkflowEdge> {
        self.outgoing(id).iter().filter(|e| e.source_handle == handle).copied().collect()
    }
}

fn display_name(node: &WorkflowNode) -> String {
    if node.label.trim().is_empty() {
        node.id.clone()
    } else {
        node.label.clone()
    }
}

// WHAT:  Every combination of iterations of the given Repeat nodes, the first
//        Repeat being the outermost loop. No Repeat means one pass with no indices.
pub fn passes(repeats: &[(String, u32)]) -> Vec<PassIndex> {
    let mut out: Vec<PassIndex> = vec![HashMap::new()];
    for (id, count) in repeats {
        let mut next = Vec::with_capacity(out.len() * (*count as usize));
        for base in &out {
            for i in 0..*count {
                let mut pass = base.clone();
                pass.insert(id.clone(), (i, *count));
                next.push(pass);
            }
        }
        out = next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::workflow::{QueryConfig, RepeatConfig, ScheduleConfig};

    fn node(id: &str, config: NodeConfig) -> WorkflowNode {
        WorkflowNode { id: id.into(), x: 0.0, y: 0.0, label: String::new(), continue_on_error: false, config }
    }

    fn query(id: &str) -> WorkflowNode {
        node(id, NodeConfig::Query(QueryConfig::default()))
    }

    fn edge(a: &str, b: &str) -> WorkflowEdge {
        WorkflowEdge { id: format!("{a}-{b}"), source: a.into(), target: b.into(), source_handle: None }
    }

    fn set(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn levels_group_independent_nodes() {
        let nodes = vec![query("a"), query("b"), query("c"), query("d")];
        let edges = vec![edge("a", "c"), edge("b", "c"), edge("c", "d")];
        let graph = Graph::new(&nodes, &edges).unwrap();
        let levels = graph.levels(&set(&["a", "b", "c", "d"])).unwrap();
        assert_eq!(levels, vec![vec!["a".to_string(), "b".to_string()], vec!["c".to_string()], vec!["d".to_string()]]);
    }

    #[test]
    fn a_cycle_is_an_error() {
        let nodes = vec![query("a"), query("b"), query("c")];
        let edges = vec![edge("a", "b"), edge("b", "c"), edge("c", "a")];
        let graph = Graph::new(&nodes, &edges).unwrap();
        let err = graph.levels(&set(&["a", "b", "c"])).err();
        assert!(matches!(err, Some(AppError::InvalidInput { .. })));
        assert!(graph.would_cycle("c", "a"));
        assert!(!Graph::new(&nodes, &edges[..2]).unwrap().would_cycle("a", "c"));
    }

    #[test]
    fn ancestors_and_leaves() {
        let nodes = vec![query("s"), query("a"), query("b"), query("x"), node("n", NodeConfig::Note(Default::default()))];
        let edges = vec![edge("s", "a"), edge("a", "b")];
        let graph = Graph::new(&nodes, &edges).unwrap();
        assert_eq!(graph.ancestors(&["b".to_string()]), set(&["s", "a", "b"]));
        assert_eq!(graph.leaves(), vec!["b".to_string(), "x".to_string()], "notes are never run");
        assert_eq!(graph.leaves_downstream_of("s"), vec!["b".to_string()]);
        assert_eq!(graph.leaves_downstream_of("x"), vec!["x".to_string()]);
    }

    #[test]
    fn dangling_edges_are_dropped_and_duplicate_ids_rejected() {
        let nodes = vec![query("a")];
        let edges = vec![edge("a", "gone")];
        assert!(Graph::new(&nodes, &edges).unwrap().edges().is_empty());
        let twice = vec![query("a"), query("a")];
        assert!(Graph::new(&twice, &[]).is_err());
        let selfie = vec![query("a")];
        assert!(Graph::new(&selfie, &[edge("a", "a")]).is_err());
    }

    #[test]
    fn leaves_under_the_same_repeats_share_a_group() {
        let nodes = vec![
            node("r", NodeConfig::Repeat(RepeatConfig { count: 3 })),
            query("q"),
            query("l1"),
            query("l2"),
            node("t", NodeConfig::Schedule(ScheduleConfig::default())),
            query("l3"),
        ];
        let edges = vec![edge("r", "q"), edge("q", "l1"), edge("q", "l2"), edge("t", "l3")];
        let graph = Graph::new(&nodes, &edges).unwrap();
        let groups = graph.plan(&graph.leaves()).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].targets, vec!["l1".to_string(), "l2".to_string()]);
        assert_eq!(groups[0].repeats, vec![("r".to_string(), 3)]);
        assert_eq!(groups[0].nodes, set(&["r", "q", "l1", "l2"]));
        assert!(groups[1].repeats.is_empty());
        assert_eq!(groups[1].nodes, set(&["t", "l3"]));
    }

    #[test]
    fn nested_repeats_expand_to_the_product() {
        let repeats = vec![("a".to_string(), 2), ("b".to_string(), 3)];
        let all = passes(&repeats);
        assert_eq!(all.len(), 6);
        assert_eq!(all[0]["a"], (0, 2));
        assert_eq!(all[0]["b"], (0, 3));
        assert_eq!(all[1]["b"], (1, 3));
        assert_eq!(all[3]["a"], (1, 2));
        assert_eq!(passes(&[]).len(), 1, "no Repeat is one plain pass");
    }

    #[test]
    fn repeat_limits_are_enforced() {
        let nodes = vec![
            node("a", NodeConfig::Repeat(RepeatConfig { count: 1000 })),
            node("b", NodeConfig::Repeat(RepeatConfig { count: 1000 })),
            query("q"),
        ];
        let edges = vec![edge("a", "q"), edge("b", "q")];
        assert!(Graph::new(&nodes, &edges).unwrap().plan(&["q".to_string()]).is_err());
        let zero = vec![node("a", NodeConfig::Repeat(RepeatConfig { count: 0 })), query("q")];
        assert!(Graph::new(&zero, &[edge("a", "q")]).unwrap().plan(&["q".to_string()]).is_err());
        assert!(Graph::new(&zero, &[]).unwrap().plan(&["nope".to_string()]).is_err());
    }

    #[test]
    fn branch_connections_must_name_their_side_and_only_branches_may() {
        let branch = node("b", NodeConfig::Branch(Default::default()));
        let nodes = vec![query("a"), branch, query("c")];
        let all: HashSet<&str> = ["a", "b", "c"].into_iter().collect();
        let mut bare = edge("b", "c");
        let graph = Graph::new(&nodes, std::slice::from_ref(&bare)).unwrap();
        assert!(graph.wiring_problem(&all).unwrap().contains("True or the False"));
        let outside: HashSet<&str> = ["a"].into_iter().collect();
        assert!(graph.wiring_problem(&outside).is_none(), "a mistake outside the run is not this run's problem");
        bare.source_handle = Some(SourceHandle::False);
        assert!(Graph::new(&nodes, &[bare]).unwrap().wiring_problem(&all).is_none());
        let mut sided = edge("a", "b");
        sided.source_handle = Some(SourceHandle::True);
        assert!(Graph::new(&nodes, &[sided]).unwrap().wiring_problem(&all).unwrap().contains("single output"));
    }

    #[test]
    fn branch_edges_are_found_by_handle() {
        let nodes = vec![query("a"), query("b"), query("c")];
        let mut yes = edge("a", "b");
        yes.source_handle = Some(SourceHandle::True);
        let mut no = edge("a", "c");
        no.source_handle = Some(SourceHandle::False);
        let edges = vec![yes, no];
        let graph = Graph::new(&nodes, &edges).unwrap();
        assert_eq!(graph.edges_from("a", Some(SourceHandle::True))[0].target, "b");
        assert_eq!(graph.edges_from("a", Some(SourceHandle::False))[0].target, "c");
        assert!(graph.edges_from("a", None).is_empty());
    }
}
