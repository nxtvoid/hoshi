//! What the refs of a snapshot name. A page reuses and replaces its nodes as it renders, so a
//! ref is the element the agent read, not the node it was: it is found again before use.

use serde_json::Value;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Roles whose text is typed or pressed, which says nothing lasting about what is around it.
const PASSING_ROLES: [&str; 5] = ["button", "textbox", "searchbox", "combobox", "spinbutton"];
/// Roles the tree has for text, which nothing acts on.
const TEXT_ROLES: [&str; 3] = ["StaticText", "InlineTextBox", "LineBreak"];
const ROOT_ROLES: [&str; 2] = ["RootWebArea", "WebArea"];
const MAX_PIECES: usize = 12;
const MAX_VISITED: usize = 400;
/// Refs kept at once; past it, the ones the latest snapshot doesn't show go.
const MAX_KEPT: usize = 5_000;

pub(super) fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn role_of(node: &Value) -> String {
    text_of(&node["role"]["value"])
}

fn name_of(node: &Value) -> String {
    text_of(&node["name"]["value"]).trim().to_owned()
}

fn is_editable(node: &Value) -> bool {
    node["properties"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|property| {
            property["name"] == "editable" && !text_of(&property["value"]["value"]).is_empty()
        })
}

/// Why a ref names nothing to act on.
#[derive(Debug, PartialEq)]
pub(super) enum Lost {
    /// It isn't there now, and may be in a moment.
    Missing(String),
    /// Waiting won't bring it back.
    Changed(String),
}

impl Lost {
    pub fn message(self) -> String {
        match self {
            Self::Missing(message) | Self::Changed(message) => message,
        }
    }
}

/// Where a page is.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Place {
    /// Tells the document shown from the ones before it, whose nodes may have had the same ids.
    pub document: String,
    pub url: String,
}

/// What an element reads as.
#[derive(Debug, Clone, PartialEq)]
struct Identity {
    role: String,
    name: String,
    /// What is said around it, or what it says itself when it has no name.
    around: Arc<[String]>,
}

impl Identity {
    fn label(&self) -> String {
        if self.name.is_empty() {
            self.role.clone()
        } else {
            format!("{} {:?}", self.role, self.name)
        }
    }

    /// Whether what is around still starts the same, apart from a number, and whether it keeps
    /// half of what it said.
    fn compare_around(&self, now: &Self) -> (bool, bool) {
        let words = |pieces: &[String]| -> HashSet<String> {
            pieces
                .iter()
                .flat_map(|piece| piece.split_whitespace())
                .map(str::to_owned)
                .collect()
        };
        let start = |pieces: &[String]| {
            pieces
                .iter()
                .find(|piece| piece.chars().any(char::is_alphabetic))
                .or(pieces.first())
                .cloned()
        };
        let (before, after) = (words(&self.around), words(&now.around));
        (
            start(&self.around) == start(&now.around),
            before.intersection(&after).count() * 2 >= before.len(),
        )
    }
}

/// The name without its numbers: what the elements a page repeats have in common.
fn skeleton(name: &str) -> String {
    name.chars().filter(|one| !one.is_ascii_digit()).collect()
}

/// An accessibility tree, to look around its nodes.
pub(super) struct Tree<'a> {
    nodes: &'a [Value],
    by_id: HashMap<&'a str, &'a Value>,
    by_backend: HashMap<i64, &'a Value>,
    /// Nodes there but kept from assistive technology, whose reading can't be told.
    ignored: HashSet<i64>,
    /// The nodes with each role and name.
    named: HashMap<(String, String), Vec<i64>>,
    around: RefCell<HashMap<i64, Arc<[String]>>>,
}

impl<'a> Tree<'a> {
    pub fn new(nodes: &'a [Value]) -> Self {
        let by_id = nodes
            .iter()
            .filter_map(|node| Some((node["nodeId"].as_str()?, node)))
            .collect();
        let mut by_backend = HashMap::new();
        let mut ignored = HashSet::new();
        let mut named: HashMap<_, Vec<i64>> = HashMap::new();
        for node in nodes {
            let role = role_of(node);
            let backend = node["backendDOMNodeId"].as_i64();
            if node["ignored"] == true {
                ignored.extend(backend);
            } else if let Some(backend) = backend.filter(|_| !TEXT_ROLES.contains(&role.as_str())) {
                by_backend.insert(backend, node);
                named
                    .entry((role, name_of(node)))
                    .or_default()
                    .push(backend);
            }
        }
        Self {
            nodes,
            by_id,
            by_backend,
            ignored,
            named,
            around: RefCell::default(),
        }
    }

    pub fn root(&self) -> Option<&'a Value> {
        self.nodes.first()
    }

    pub fn children(&self, node: &'a Value) -> impl DoubleEndedIterator<Item = &'a Value> {
        node["childIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|id| self.by_id.get(id.as_str()?).copied())
    }

    fn parent(&self, node: &'a Value) -> Option<&'a Value> {
        self.by_id.get(node["parentId"].as_str()?).copied()
    }

    /// What `node` says, piece by piece, apart from what `besides` does.
    fn pieces(&self, node: &'a Value, besides: Option<&'a Value>) -> Vec<String> {
        let mut pieces = Vec::new();
        let mut stack = vec![node];
        let mut visited = 0;
        while let Some(current) = stack.pop() {
            visited += 1;
            if visited > MAX_VISITED || pieces.len() >= MAX_PIECES {
                break;
            }
            if besides.is_some_and(|other| std::ptr::eq(other, current)) {
                continue;
            }
            let role = role_of(current);
            if role == "StaticText" {
                let text = name_of(current);
                if !text.is_empty() {
                    pieces.push(text);
                }
                continue;
            }
            if besides.is_some() && (PASSING_ROLES.contains(&role.as_str()) || is_editable(current))
            {
                continue;
            }
            stack.extend(self.children(current).rev());
        }
        pieces
    }

    fn around(&self, backend: i64, node: &'a Value) -> Arc<[String]> {
        if let Some(known) = self.around.borrow().get(&backend) {
            return known.clone();
        }
        let role = role_of(node);
        let mut pieces = Vec::new();
        if name_of(node).is_empty() && !PASSING_ROLES.contains(&role.as_str()) && !is_editable(node)
        {
            pieces = self.pieces(node, None);
        }
        let mut current = node;
        while pieces.is_empty()
            && let Some(parent) = self.parent(current)
        {
            pieces = self.pieces(parent, Some(node));
            current = parent;
        }
        let pieces: Arc<[String]> = pieces.into();
        self.around.borrow_mut().insert(backend, pieces.clone());
        pieces
    }

    /// What the node with this id reads as, when it is shown.
    fn identity(&self, backend: i64) -> Option<Identity> {
        let node = self.by_backend.get(&backend)?;
        Some(Identity {
            role: role_of(node),
            name: name_of(node),
            around: self.around(backend, node),
        })
    }

    fn named_as(&self, identity: &Identity) -> &[i64] {
        self.named
            .get(&(identity.role.clone(), identity.name.clone()))
            .map_or(&[], Vec::as_slice)
    }

    /// Up to two nodes that read as `identity`.
    fn reading_as(&self, identity: &Identity) -> Vec<i64> {
        self.named_as(identity)
            .iter()
            .copied()
            .filter(|node| {
                self.by_backend
                    .get(node)
                    .is_some_and(|value| self.around(*node, value) == identity.around)
            })
            .take(2)
            .collect()
    }
}

/// An element as a snapshot showed it.
#[derive(Debug, Clone)]
struct Target {
    node: i64,
    identity: Identity,
    /// Whether others read the same, so it can only be told from them by its node.
    twins: bool,
    /// Whether nothing else had its role and name, which alone tell it then.
    only: bool,
    url: String,
    generation: u64,
}

/// The refs given so far. A number is never given twice, so an old ref never names another
/// element.
#[derive(Default)]
pub(super) struct Refs {
    targets: HashMap<String, Target>,
    /// The ref each node had in the latest snapshot.
    by_node: HashMap<i64, String>,
    given: usize,
    generation: u64,
    place: Place,
}

impl Refs {
    /// A new snapshot of the page at `place` begins.
    pub fn begin(&mut self, tree: &Tree, place: Place) {
        if self.place.document != place.document {
            self.clear();
        }
        let mut by_node = HashMap::new();
        let mut contested = HashSet::new();
        for (reference, target) in &self.targets {
            let Ok(node) = self.locate(reference, target, tree, &place.url) else {
                continue;
            };
            let stays = node == target.node;
            match by_node.get(&node) {
                Some((_, true)) if !stays => {}
                Some((_, false)) if !stays => {
                    contested.insert(node);
                }
                _ => {
                    by_node.insert(node, (reference.clone(), stays));
                }
            }
        }
        self.by_node = by_node
            .into_iter()
            .filter(|(node, (_, stays))| *stays || !contested.contains(node))
            .map(|(node, (reference, _))| (node, reference))
            .collect();
        self.place = place;
        self.generation += 1;
    }

    /// The page went elsewhere: what was read is gone.
    pub fn clear(&mut self) {
        self.targets.clear();
        self.by_node.clear();
    }

    /// The ref of `node`, the one it had while it is the same element.
    pub fn give(&mut self, tree: &Tree, node: &Value) -> Option<String> {
        let backend = node["backendDOMNodeId"].as_i64()?;
        let identity = tree.identity(backend)?;
        let reference = match self.by_node.get(&backend) {
            Some(reference) => reference.clone(),
            None => {
                self.given += 1;
                format!("e{}", self.given)
            }
        };
        let target = Target {
            node: backend,
            twins: tree.reading_as(&identity).len() > 1,
            only: tree.named_as(&identity).len() == 1,
            identity,
            url: self.place.url.clone(),
            generation: self.generation,
        };
        self.by_node.insert(backend, reference.clone());
        self.targets.insert(reference.clone(), target);
        Some(reference)
    }

    /// Lets go of the refs no snapshot showed lately, once there are too many.
    pub fn trim(&mut self) {
        if self.targets.len() <= MAX_KEPT {
            return;
        }
        let latest = self.generation;
        self.targets.retain(|_, target| target.generation == latest);
        let targets = &self.targets;
        self.by_node
            .retain(|_, reference| targets.contains_key(reference));
    }

    /// What `reference` names on the page at `place`.
    fn target(&self, reference: &str, place: &Place) -> Result<&Target, Lost> {
        let target = self.targets.get(reference).ok_or_else(|| {
            Lost::Changed(format!(
                "no element {reference} in the last snapshot: take a new snapshot"
            ))
        })?;
        if self.place.document != place.document {
            return Err(Lost::Changed(format!(
                "the page that had {reference} is gone: take a new snapshot"
            )));
        }
        Ok(target)
    }

    /// The node `reference` names when how that node reads now settles it, as `read` tells.
    pub fn find_alone(
        &self,
        reference: &str,
        place: &Place,
        read: impl FnOnce(i64) -> Option<Value>,
    ) -> Result<Option<i64>, Lost> {
        let target = self.target(reference, place)?;
        let same = target.only
            && read(target.node).is_some_and(|node| {
                node["ignored"] != true
                    && role_of(&node) == target.identity.role
                    && name_of(&node) == target.identity.name
            });
        Ok(same.then_some(target.node))
    }

    /// The node `reference` names in the page as `tree` shows it now, at `place`.
    pub fn find(&self, reference: &str, tree: &Tree, place: &Place) -> Result<i64, Lost> {
        let target = self.target(reference, place)?;
        self.locate(reference, target, tree, &place.url).map_err(|now| {
            let label = target.identity.label();
            match now {
                Elsewhere::Reused(now) => Lost::Changed(format!(
                    "{reference} was {label} {} and its place is now {} {}: take a new snapshot",
                    near(&target.identity),
                    now.label(),
                    near(&now)
                )),
                Elsewhere::Several => Lost::Changed(format!(
                    "the page replaced {reference} ({label}) and has several like it: take a new snapshot"
                )),
                Elsewhere::Gone => Lost::Missing(format!(
                    "{reference} ({label}) isn't on the page any more, or is hidden, as behind a dialog: take a new snapshot"
                )),
            }
        })
    }

    /// The node that is the target now.
    fn locate(
        &self,
        reference: &str,
        target: &Target,
        tree: &Tree,
        url: &str,
    ) -> Result<i64, Elsewhere> {
        if ROOT_ROLES.contains(&target.identity.role.as_str()) {
            return Ok(target.node);
        }
        let now = tree.identity(target.node);
        // A later snapshot told it from what its node is now.
        let another = self
            .by_node
            .get(&target.node)
            .is_some_and(|other| other != reference);
        if now.as_ref() == Some(&target.identity) && !another {
            return Ok(target.node);
        }
        // Others that read as it did, when nothing did before: the element itself, moved.
        let alike = if target.twins || target.url != url {
            Vec::new()
        } else {
            tree.reading_as(&target.identity)
        };
        let Some(now) = now else {
            return match alike[..] {
                [only] => Ok(only),
                [] if tree.ignored.contains(&target.node) => Ok(target.node),
                [] => Err(Elsewhere::Gone),
                _ => Err(Elsewhere::Several),
            };
        };
        let reused = another || self.was_elsewhere(&now, target, tree);
        match alike[..] {
            [only] if reused || self.read_otherwise(only, target) => return Ok(only),
            [_, _, ..] if reused => return Err(Elsewhere::Several),
            _ if reused => return Err(Elsewhere::Reused(now)),
            _ => {}
        }
        let (starts, keeps) = target.identity.compare_around(&now);
        let same = now.name == target.identity.name && starts && keeps;
        let kind = now.role == target.identity.role;
        let (mates, changed) = self.mates(target, tree);
        // Among others like it, one whose surroundings are all new is another's.
        let elsewhere = mates > 0 && !starts && !keeps;
        if same || kind && !elsewhere && changed * 2 <= mates {
            Ok(target.node)
        } else {
            Err(Elsewhere::Reused(now))
        }
    }

    /// Whether another element read as `now` when `target` did and its node no longer does:
    /// the page moved that one here.
    fn was_elsewhere(&self, now: &Identity, target: &Target, tree: &Tree) -> bool {
        self.targets.values().any(|other| {
            other.generation == target.generation
                && other.node != target.node
                && other.identity == *now
                && tree.identity(other.node).as_ref() != Some(now)
        })
    }

    /// Whether `node` read as something else when `target` was read.
    fn read_otherwise(&self, node: i64, target: &Target) -> bool {
        self.targets.values().any(|other| {
            other.generation == target.generation
                && other.node == node
                && other.identity != target.identity
        })
    }

    /// How many elements are like `target`, and how many of them changed with it, as most do
    /// when a list scrolls or sorts over the same nodes.
    fn mates(&self, target: &Target, tree: &Tree) -> (usize, usize) {
        let name = skeleton(&target.identity.name);
        let (mut mates, mut changed) = (0, 0);
        for other in self.targets.values() {
            if other.generation != target.generation
                || other.node == target.node
                || other.identity.role != target.identity.role
                || skeleton(&other.identity.name) != name
            {
                continue;
            }
            if let Some(now) = tree.identity(other.node) {
                mates += 1;
                changed += usize::from(now != other.identity);
            }
        }
        (mates, changed)
    }
}

/// What became of an element that isn't its node any more.
#[derive(Debug, PartialEq)]
enum Elsewhere {
    Reused(Identity),
    Several,
    Gone,
}

fn near(identity: &Identity) -> String {
    let said: String = identity.around.join(" ").chars().take(60).collect();
    if said.is_empty() {
        String::new()
    } else {
        format!("(in {said:?})")
    }
}

#[cfg(test)]
mod tests;
