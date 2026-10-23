//! A* over triangle corridors with a bounded node pool and a sliced (resumable) API.
//!
//! A node is a triangle entered through one specific link, at that link's crossing point (the
//! middle of the crossable stretch of the edge; a ledge link lands at the middle of its target
//! edge). Cost is the distance walked inside each triangle scaled by the filter's multiplier for
//! it, plus, for a ledge, the distance of the jump scaled by the landing triangle's multiplier.
//! Because the entry point is part of the node, the cost of leaving a triangle is exact and the
//! search is cost-optimal for this model.
//!
//! A triangle on a stale reference is a wall: it is never expanded, and `finalize` cuts a
//! corridor at the first stale triangle.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use glam::Vec3;

use super::NavPoint;
use crate::navmesh::{NavRef, NavWorld, QueryFilter};

const NO_PARENT: u32 = u32::MAX;
const HEURISTIC_SCALE: f32 = 0.999;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completeness {
    Complete,
    /// The goal is unreachable under the filter; the path ends at the explored triangle closest
    /// to it, or was cut short because triangles unloaded after the search.
    Partial,
    /// The node pool ran out before the goal was reached, so the goal may still be reachable;
    /// the path ends at the explored triangle closest to it.
    NodeLimit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathResult {
    pub refs: Vec<NavRef>,
    pub completeness: Completeness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SearchError {
    #[error("start is stale or excluded by the filter")]
    InvalidStart,
    #[error("goal is stale or excluded by the filter")]
    InvalidGoal,
    #[error("start or goal position is not finite")]
    NonFinitePosition,
    #[error("search was not initialised")]
    NotInitialised,
    #[error("search has not finished")]
    Unfinished,
    #[error("the path no longer starts on a valid triangle")]
    PathInvalidated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchStatus {
    InProgress,
    Done(Completeness),
    Failed(SearchError),
}

/// How a triangle was entered: the triangle, the one before it and the link taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct NodeKey {
    target: NavRef,
    from: Option<NavRef>,
    link: u16,
}

#[derive(Debug, Clone, Copy)]
struct Node {
    nav_ref: NavRef,
    entry: Vec3,
    cost: f32,
    total: f32,
    parent: u32,
    closed: bool,
}

#[derive(Debug, Clone, Copy)]
struct OpenEntry {
    total: f32,
    node: u32,
}

impl PartialEq for OpenEntry {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl Eq for OpenEntry {}

impl PartialOrd for OpenEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OpenEntry {
    /// Reversed so the binary heap pops the smallest total cost first.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .total
            .total_cmp(&self.total)
            .then_with(|| other.node.cmp(&self.node))
    }
}

#[derive(Debug, Clone, Copy)]
struct Request {
    goal: NavPoint,
    best_node: u32,
    best_remaining: f32,
    /// The node the goal was reached through, once it was.
    goal_node: Option<u32>,
}

/// Reusable search state; keep one per worker thread.
#[derive(Debug)]
pub struct PathSearch {
    max_nodes: usize,
    nodes: Vec<Node>,
    index_of: HashMap<NodeKey, u32>,
    open: BinaryHeap<OpenEntry>,
    filter: QueryFilter,
    heuristic_scale: f32,
    node_limit_hit: bool,
    request: Option<Request>,
    status: SearchStatus,
}

impl PathSearch {
    /// `max_nodes` bounds the explored (triangle, entry link) states, not triangles.
    pub fn new(max_nodes: usize) -> Self {
        Self {
            max_nodes: max_nodes.max(1),
            nodes: Vec::new(),
            index_of: HashMap::new(),
            open: BinaryHeap::new(),
            filter: QueryFilter::default(),
            heuristic_scale: HEURISTIC_SCALE,
            node_limit_hit: false,
            request: None,
            status: SearchStatus::Failed(SearchError::NotInitialised),
        }
    }

    pub fn status(&self) -> SearchStatus {
        self.status
    }

    pub fn init(
        &mut self,
        world: &NavWorld,
        start: NavPoint,
        goal: NavPoint,
        filter: &QueryFilter,
    ) -> Result<(), SearchError> {
        self.nodes.clear();
        self.index_of.clear();
        self.open.clear();
        self.request = None;
        self.node_limit_hit = false;
        self.status = SearchStatus::Failed(SearchError::NotInitialised);
        if !start.pos.is_finite() || !goal.pos.is_finite() {
            return Err(SearchError::NonFinitePosition);
        }
        if !is_usable(world, filter, start.nav_ref) {
            return Err(SearchError::InvalidStart);
        }
        if !is_usable(world, filter, goal.nav_ref) {
            return Err(SearchError::InvalidGoal);
        }
        self.filter = filter.clone();
        self.heuristic_scale = HEURISTIC_SCALE * filter.min_cost_multiplier();
        let remaining = start.pos.distance(goal.pos);
        let start_total = remaining * self.heuristic_scale;
        self.insert_node(
            NodeKey {
                target: start.nav_ref,
                from: None,
                link: 0,
            },
            Node {
                nav_ref: start.nav_ref,
                entry: start.pos,
                cost: 0.0,
                total: start_total,
                parent: NO_PARENT,
                closed: false,
            },
        );
        self.open.push(OpenEntry {
            total: start_total,
            node: 0,
        });
        let reached = start.nav_ref == goal.nav_ref;
        self.request = Some(Request {
            goal,
            best_node: 0,
            best_remaining: remaining,
            goal_node: reached.then_some(0),
        });
        self.status = if reached {
            SearchStatus::Done(Completeness::Complete)
        } else {
            SearchStatus::InProgress
        };
        Ok(())
    }

    /// Expands up to `max_iterations` nodes.
    pub fn update(&mut self, world: &NavWorld, max_iterations: usize) -> SearchStatus {
        for _ in 0..max_iterations {
            if self.status != SearchStatus::InProgress {
                break;
            }
            self.status = self.step(world);
        }
        self.status
    }

    /// Runs the search to completion in one call.
    pub fn run(&mut self, world: &NavWorld) -> SearchStatus {
        self.update(world, usize::MAX)
    }

    /// Extracts the corridor, dropping everything from the first triangle that went stale
    /// since the search ran (the result is then `Partial`).
    pub fn finalize(&self, world: &NavWorld) -> Result<PathResult, SearchError> {
        let mut refs = Vec::new();
        let completeness = self.finalize_into(world, &mut refs)?;
        Ok(PathResult { refs, completeness })
    }

    /// [`Self::finalize`] writing into a caller-owned buffer, which is cleared first.
    pub fn finalize_into(
        &self,
        world: &NavWorld,
        refs: &mut Vec<NavRef>,
    ) -> Result<Completeness, SearchError> {
        refs.clear();
        let SearchStatus::Done(completeness) = self.status else {
            return Err(match self.status {
                SearchStatus::Failed(error) => error,
                _ => SearchError::Unfinished,
            });
        };
        let request = self.request.ok_or(SearchError::NotInitialised)?;
        let mut cursor = match (completeness, request.goal_node) {
            (Completeness::Complete, Some(goal_node)) => goal_node,
            _ => request.best_node,
        };
        while cursor != NO_PARENT {
            let node = &self.nodes[cursor as usize];
            refs.push(node.nav_ref);
            cursor = node.parent;
        }
        refs.reverse();
        let valid_prefix = refs.iter().take_while(|r| world.is_valid(**r)).count();
        if valid_prefix == 0 {
            refs.clear();
            return Err(SearchError::PathInvalidated);
        }
        let trimmed = valid_prefix < refs.len();
        refs.truncate(valid_prefix);
        Ok(match (trimmed, completeness) {
            (true, Completeness::Complete) => Completeness::Partial,
            _ => completeness,
        })
    }

    fn insert_node(&mut self, key: NodeKey, node: Node) -> u32 {
        let index = self.nodes.len() as u32;
        self.index_of.insert(key, index);
        self.nodes.push(node);
        index
    }

    fn step(&mut self, world: &NavWorld) -> SearchStatus {
        let Some(request) = self.request else {
            return SearchStatus::Failed(SearchError::NotInitialised);
        };
        let Some(entry) = self.open.pop() else {
            return SearchStatus::Done(self.exhausted_completeness());
        };
        let node = self.nodes[entry.node as usize];
        if node.closed || entry.total > node.total || !world.is_valid(node.nav_ref) {
            return SearchStatus::InProgress;
        }
        self.nodes[entry.node as usize].closed = true;
        if node.nav_ref == request.goal.nav_ref {
            self.request = Some(Request {
                goal_node: Some(entry.node),
                ..request
            });
            return SearchStatus::Done(Completeness::Complete);
        }
        let remaining = node.entry.distance(request.goal.pos);
        if remaining < request.best_remaining {
            self.request = Some(Request {
                best_node: entry.node,
                best_remaining: remaining,
                ..request
            });
        }
        self.expand(world, entry.node, request.goal);
        SearchStatus::InProgress
    }

    fn exhausted_completeness(&self) -> Completeness {
        if self.node_limit_hit {
            Completeness::NodeLimit
        } else {
            Completeness::Partial
        }
    }

    fn expand(&mut self, world: &NavWorld, current_index: u32, goal: NavPoint) {
        let current = self.nodes[current_index as usize];
        let Some(mesh) = world.mesh(current.nav_ref) else {
            return;
        };
        let Some(current_multiplier) = self
            .filter
            .traversal_multiplier(mesh, current.nav_ref.tri())
        else {
            return;
        };
        let parent_ref =
            (current.parent != NO_PARENT).then(|| self.nodes[current.parent as usize].nav_ref);
        for (index, link) in world.links(current.nav_ref).enumerate() {
            if Some(link.target) == parent_ref || !self.filter.allows_link(link.link_type) {
                continue;
            }
            let Some(target_mesh) = world.mesh(link.target) else {
                continue;
            };
            let Some(target_multiplier) = self
                .filter
                .traversal_multiplier(target_mesh, link.target.tri())
            else {
                continue;
            };
            let crossing = link.crossing();
            let mut cost = current.cost + current.entry.distance(crossing) * current_multiplier;
            let entry = match link.landing {
                Some(landing) => {
                    cost += crossing.distance(landing) * target_multiplier;
                    landing
                }
                None => crossing,
            };
            let remaining = if link.target == goal.nav_ref {
                cost += entry.distance(goal.pos) * target_multiplier;
                0.0
            } else {
                entry.distance(goal.pos) * self.heuristic_scale
            };
            let key = NodeKey {
                target: link.target,
                from: Some(current.nav_ref),
                link: u16::try_from(index).unwrap_or(u16::MAX),
            };
            self.relax(key, entry, cost, cost + remaining, current_index);
        }
    }

    fn relax(&mut self, key: NodeKey, entry: Vec3, cost: f32, total: f32, parent: u32) {
        let candidate = Node {
            nav_ref: key.target,
            entry,
            cost,
            total,
            parent,
            closed: false,
        };
        let index = match self.index_of.get(&key) {
            Some(&existing) => {
                // Written so that a NaN total never counts as an improvement.
                if !(total < self.nodes[existing as usize].total) {
                    return;
                }
                self.nodes[existing as usize] = candidate;
                existing
            }
            None if self.nodes.len() >= self.max_nodes => {
                self.node_limit_hit = true;
                return;
            }
            None => self.insert_node(key, candidate),
        };
        self.open.push(OpenEntry { total, node: index });
    }
}

fn is_usable(world: &NavWorld, filter: &QueryFilter, nav_ref: NavRef) -> bool {
    world
        .mesh(nav_ref)
        .is_some_and(|mesh| filter.is_passable(mesh, nav_ref.tri()))
}

/// Convenience wrapper: init, run to completion and finalize.
pub fn find_path(
    world: &NavWorld,
    search: &mut PathSearch,
    start: NavPoint,
    goal: NavPoint,
    filter: &QueryFilter,
) -> Result<PathResult, SearchError> {
    search.init(world, start, goal, filter)?;
    search.run(world);
    search.finalize(world)
}
