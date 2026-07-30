//! The vault graph and its force-directed layout.
//!
//! Obsidian's graph view is the feature people open the app for, and it has to
//! stay responsive while the layout is still settling. Two decisions follow
//! from that:
//!
//! - Repulsion uses a **Barnes-Hut quadtree**, making a step O(n log n) instead
//!   of O(n²). A five-thousand-note vault stays interactive.
//! - The simulation **detects when it has settled** and stops. An idle graph
//!   costs no CPU, which matters for a program that may sit open all day.
//!
//! Layout is deterministic: nodes start on a fixed spiral rather than at random
//! positions, so reopening the graph shows the same picture and tests can
//! assert on it.

use std::collections::VecDeque;

use crate::index::{LinkTarget, NoteId, VaultIndex};

/// A 2-D point or vector in graph space.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    #[must_use]
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    #[must_use]
    pub fn length(self) -> f32 {
        self.x.hypot(self.y)
    }
}

/// What a graph node represents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    /// A note in the vault.
    Note(NoteId),
    /// A link target with no note behind it. Obsidian draws these hollow, and
    /// they're often the most useful thing on the graph: they're the notes you
    /// meant to write.
    Unresolved,
    Tag,
    Attachment,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub label: String,
    pub kind: NodeKind,
    pub pos: Vec2,
    pub vel: Vec2,
    /// Number of edges, which drives node size as it does in Obsidian.
    pub degree: usize,
    /// Held in place by a drag; excluded from force integration.
    pub pinned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
}

/// What to include when building the graph.
#[derive(Debug, Clone)]
pub struct GraphOptions {
    pub show_unresolved: bool,
    pub show_tags: bool,
    pub show_attachments: bool,
    /// Include notes with no links at all.
    pub show_orphans: bool,
    /// Only include notes carrying this tag (or one nested under it).
    pub tag_filter: Option<String>,
}

impl Default for GraphOptions {
    fn default() -> Self {
        Self {
            show_unresolved: true,
            show_tags: false,
            show_attachments: false,
            show_orphans: true,
            tag_filter: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    adjacency: Vec<Vec<usize>>,
}

impl Graph {
    /// Builds the graph for a vault.
    #[must_use]
    pub fn build(index: &VaultIndex, options: &GraphOptions) -> Self {
        let mut nodes: Vec<Node> = Vec::new();
        let mut edges: Vec<Edge> = Vec::new();
        // Note id → graph node index, for the notes that passed the filters.
        let mut note_nodes: Vec<Option<usize>> = vec![None; index.len()];
        let mut named: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

        let included = |note: &crate::index::IndexedNote| match &options.tag_filter {
            None => true,
            Some(filter) => note
                .tags
                .iter()
                .any(|t| t == filter || t.starts_with(&format!("{filter}/"))),
        };

        for (id, note) in index.notes().iter().enumerate() {
            if !included(note) {
                continue;
            }
            note_nodes[id] = Some(nodes.len());
            nodes.push(Node {
                label: note.meta.title.clone(),
                kind: NodeKind::Note(id),
                pos: Vec2::default(),
                vel: Vec2::default(),
                degree: 0,
                pinned: false,
            });
        }

        let add_edge = |edges: &mut Vec<Edge>, from: usize, to: usize| {
            if from == to {
                return;
            }
            // The graph is undirected for layout purposes, so a mutual link is
            // one edge, not two overlapping ones.
            if edges
                .iter()
                .any(|e| (e.from == from && e.to == to) || (e.from == to && e.to == from))
            {
                return;
            }
            edges.push(Edge { from, to });
        };

        for (id, note) in index.notes().iter().enumerate() {
            let Some(from) = note_nodes[id] else { continue };

            for link in &note.links {
                match &link.target {
                    LinkTarget::Note(target) => {
                        if let Some(to) = note_nodes.get(*target).copied().flatten() {
                            add_edge(&mut edges, from, to);
                        }
                    }
                    LinkTarget::Unresolved(name) if options.show_unresolved => {
                        let key = format!("unresolved:{}", name.to_lowercase());
                        let to = *named.entry(key).or_insert_with(|| {
                            nodes.push(Node {
                                label: name.clone(),
                                kind: NodeKind::Unresolved,
                                pos: Vec2::default(),
                                vel: Vec2::default(),
                                degree: 0,
                                pinned: false,
                            });
                            nodes.len() - 1
                        });
                        add_edge(&mut edges, from, to);
                    }
                    LinkTarget::Attachment(path) if options.show_attachments => {
                        let key = format!("attachment:{}", path.to_lowercase());
                        let to = *named.entry(key).or_insert_with(|| {
                            nodes.push(Node {
                                label: path.rsplit('/').next().unwrap_or(path).to_string(),
                                kind: NodeKind::Attachment,
                                pos: Vec2::default(),
                                vel: Vec2::default(),
                                degree: 0,
                                pinned: false,
                            });
                            nodes.len() - 1
                        });
                        add_edge(&mut edges, from, to);
                    }
                    _ => {}
                }
            }

            if options.show_tags {
                for tag in &note.tags {
                    let key = format!("tag:{}", tag.to_lowercase());
                    let to = *named.entry(key).or_insert_with(|| {
                        nodes.push(Node {
                            label: format!("#{tag}"),
                            kind: NodeKind::Tag,
                            pos: Vec2::default(),
                            vel: Vec2::default(),
                            degree: 0,
                            pinned: false,
                        });
                        nodes.len() - 1
                    });
                    add_edge(&mut edges, from, to);
                }
            }
        }

        let mut graph = Self {
            nodes,
            edges,
            adjacency: Vec::new(),
        };
        graph.rebuild_adjacency();

        if !options.show_orphans {
            graph.remove_orphans();
        }

        graph.seed_positions();
        graph
    }

    fn rebuild_adjacency(&mut self) {
        self.adjacency = vec![Vec::new(); self.nodes.len()];
        for node in &mut self.nodes {
            node.degree = 0;
        }
        for edge in &self.edges {
            self.adjacency[edge.from].push(edge.to);
            self.adjacency[edge.to].push(edge.from);
            self.nodes[edge.from].degree += 1;
            self.nodes[edge.to].degree += 1;
        }
    }

    fn remove_orphans(&mut self) {
        let keep: Vec<bool> = self.nodes.iter().map(|n| n.degree > 0).collect();
        if keep.iter().all(|&k| k) {
            return;
        }

        let mut remap = vec![usize::MAX; self.nodes.len()];
        let mut next = 0;
        for (i, &k) in keep.iter().enumerate() {
            if k {
                remap[i] = next;
                next += 1;
            }
        }

        let mut index = 0;
        self.nodes.retain(|_| {
            let k = keep[index];
            index += 1;
            k
        });
        self.edges
            .retain(|e| remap[e.from] != usize::MAX && remap[e.to] != usize::MAX);
        for edge in &mut self.edges {
            edge.from = remap[edge.from];
            edge.to = remap[edge.to];
        }
        self.rebuild_adjacency();
    }

    /// Places nodes on a golden-angle spiral.
    ///
    /// Any deterministic, non-degenerate starting layout works; a spiral is
    /// used because it spreads nodes evenly, which lets the simulation settle
    /// in far fewer steps than a random cloud or a single point would.
    fn seed_positions(&mut self) {
        const GOLDEN_ANGLE: f32 = 2.399_963_2;
        let spread = 24.0;
        for (i, node) in self.nodes.iter_mut().enumerate() {
            let radius = spread * (i as f32).sqrt();
            let angle = GOLDEN_ANGLE * i as f32;
            node.pos = Vec2::new(radius * angle.cos(), radius * angle.sin());
            node.vel = Vec2::default();
        }
    }

    #[must_use]
    pub fn neighbors(&self, node: usize) -> &[usize] {
        self.adjacency.get(node).map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Finds the graph node for a note.
    #[must_use]
    pub fn node_of_note(&self, id: NoteId) -> Option<usize> {
        self.nodes.iter().position(|n| n.kind == NodeKind::Note(id))
    }

    /// Node indices within `depth` hops of `origin`, including it.
    ///
    /// This is Obsidian's local graph: the neighborhood of the note you're
    /// reading, which is usually far more legible than the whole vault.
    #[must_use]
    pub fn neighborhood(&self, origin: usize, depth: usize) -> Vec<usize> {
        if origin >= self.nodes.len() {
            return Vec::new();
        }
        let mut seen = vec![false; self.nodes.len()];
        let mut out = Vec::new();
        let mut queue = VecDeque::new();

        seen[origin] = true;
        queue.push_back((origin, 0usize));

        while let Some((node, d)) = queue.pop_front() {
            out.push(node);
            if d == depth {
                continue;
            }
            for &next in self.neighbors(node) {
                if !seen[next] {
                    seen[next] = true;
                    queue.push_back((next, d + 1));
                }
            }
        }

        out
    }

    /// A graph holding only `indices` and the edges that run between them.
    ///
    /// The local graph needs its own layout rather than a filtered view of the
    /// whole vault's. Laying a neighbourhood out as part of a five-thousand-note
    /// picture scatters it across coordinates the local view never shows, so it
    /// arrives as a clump in one corner of an otherwise empty pane. Positions
    /// are re-seeded, so the neighbourhood settles into the space it actually
    /// has.
    #[must_use]
    pub fn subgraph(&self, indices: &[usize]) -> Self {
        let mut remap = vec![usize::MAX; self.nodes.len()];
        let mut nodes = Vec::with_capacity(indices.len());
        for &index in indices {
            if index >= self.nodes.len() || remap[index] != usize::MAX {
                continue;
            }
            remap[index] = nodes.len();
            nodes.push(self.nodes[index].clone());
        }

        let kept = |node: usize| remap.get(node).copied().filter(|&n| n != usize::MAX);
        let edges = self
            .edges
            .iter()
            .filter_map(|edge| {
                Some(Edge {
                    from: kept(edge.from)?,
                    to: kept(edge.to)?,
                })
            })
            .collect();

        let mut graph = Self {
            nodes,
            edges,
            adjacency: Vec::new(),
        };
        // Degree is recounted over the kept edges, which is what the layout
        // should react to: a hub's pull inside the neighbourhood comes from the
        // links you can see, not from the hundred you can't.
        graph.rebuild_adjacency();
        graph.seed_positions();
        graph
    }

    /// Bounding box of all node positions as `(min_x, min_y, max_x, max_y)`.
    ///
    /// Returns a unit box for an empty graph so callers can divide by its size
    /// without a special case.
    #[must_use]
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        if self.nodes.is_empty() {
            return (-1.0, -1.0, 1.0, 1.0);
        }
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        for node in &self.nodes {
            min_x = min_x.min(node.pos.x);
            min_y = min_y.min(node.pos.y);
            max_x = max_x.max(node.pos.x);
            max_y = max_y.max(node.pos.y);
        }
        // Guard against a zero-width box when every node coincides.
        if (max_x - min_x).abs() < 1e-3 {
            min_x -= 1.0;
            max_x += 1.0;
        }
        if (max_y - min_y).abs() < 1e-3 {
            min_y -= 1.0;
            max_y += 1.0;
        }
        (min_x, min_y, max_x, max_y)
    }
}

/// Tunable force constants, exposed so the UI can offer Obsidian's graph
/// sliders.
#[derive(Debug, Clone, Copy)]
pub struct ForceParams {
    /// Strength of node-node repulsion.
    pub repel: f32,
    /// Rest length of a link.
    pub link_distance: f32,
    /// How strongly links pull.
    pub link_strength: f32,
    /// Pull toward the origin, which keeps disconnected components from
    /// drifting apart forever.
    pub center_gravity: f32,
    /// Velocity retained per step.
    pub damping: f32,
    /// Barnes-Hut accuracy: smaller is more accurate and slower.
    pub theta: f32,
    /// Integration timestep.
    pub dt: f32,
}

impl Default for ForceParams {
    fn default() -> Self {
        // Tuned for spread rather than for any particular scale: the view
        // auto-fits to the layout's bounding box, so only the ratio of
        // node spacing to graph diameter reaches the screen. `repel` is low
        // relative to that ratio because repulsion is degree-weighted (see
        // `mass_of`), and gravity is high enough to keep orphans from drifting
        // out far enough to set the zoom for everyone else.
        Self {
            repel: 300.0,
            link_distance: 32.0,
            link_strength: 0.06,
            center_gravity: 0.06,
            damping: 0.82,
            theta: 0.8,
            dt: 0.85,
        }
    }
}

/// A graph plus its running layout simulation.
#[derive(Debug, Clone)]
pub struct Simulation {
    pub graph: Graph,
    pub params: ForceParams,
    /// Total kinetic energy from the last step, used for settle detection.
    energy: f32,
    settled: bool,
    steps: usize,
}

/// Below this average speed per node, the layout is visually static.
const SETTLE_ENERGY: f32 = 0.06;
/// Hard cap so a pathological graph can't spin forever.
const MAX_STEPS: usize = 4_000;

impl Simulation {
    #[must_use]
    pub fn new(graph: Graph) -> Self {
        Self {
            graph,
            params: ForceParams::default(),
            energy: f32::MAX,
            settled: false,
            steps: 0,
        }
    }

    /// Whether the layout has stopped moving. When true, callers should stop
    /// stepping and stop redrawing.
    #[must_use]
    pub fn is_settled(&self) -> bool {
        self.settled
    }

    #[must_use]
    pub fn energy(&self) -> f32 {
        self.energy
    }

    /// Restarts the simulation, e.g. after a drag or a filter change.
    pub fn reheat(&mut self) {
        self.settled = false;
        self.energy = f32::MAX;
        self.steps = 0;
    }

    /// Advances the layout one step. A no-op once settled.
    pub fn step(&mut self) {
        if self.settled || self.graph.nodes.is_empty() {
            return;
        }

        let count = self.graph.nodes.len();
        let mut forces = vec![Vec2::default(); count];

        // Repulsion, approximated with a quadtree.
        let tree = QuadTree::build(&self.graph.nodes);
        for (index, force) in forces.iter_mut().enumerate() {
            let repulsion = tree.repulsion(
                &self.graph.nodes,
                index,
                self.params.repel,
                self.params.theta,
            );
            force.x += repulsion.x;
            force.y += repulsion.y;
        }

        // Link springs.
        //
        // A link's pull is divided by the lesser of its two endpoints' degrees.
        // Without that, every link into a hub pulls at full strength and the
        // hub's neighbourhood collapses onto it; a leaf hanging off a hub still
        // gets pulled home at full strength, because the leaf is the lesser end.
        for edge in &self.graph.edges {
            let a = self.graph.nodes[edge.from].pos;
            let b = self.graph.nodes[edge.to].pos;
            let crowding = 1.0
                + self.graph.nodes[edge.from]
                    .degree
                    .min(self.graph.nodes[edge.to].degree) as f32;
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            let dist = (dx * dx + dy * dy).sqrt().max(0.01);
            let pull = (dist - self.params.link_distance) * self.params.link_strength / crowding;
            let fx = dx / dist * pull;
            let fy = dy / dist * pull;
            forces[edge.from].x += fx;
            forces[edge.from].y += fy;
            forces[edge.to].x -= fx;
            forces[edge.to].y -= fy;
        }

        // Gravity toward the origin.
        for (i, node) in self.graph.nodes.iter().enumerate() {
            forces[i].x -= node.pos.x * self.params.center_gravity;
            forces[i].y -= node.pos.y * self.params.center_gravity;
        }

        // Integrate.
        let mut energy = 0.0;
        for (i, node) in self.graph.nodes.iter_mut().enumerate() {
            if node.pinned {
                node.vel = Vec2::default();
                continue;
            }
            node.vel.x = (node.vel.x + forces[i].x * self.params.dt) * self.params.damping;
            node.vel.y = (node.vel.y + forces[i].y * self.params.dt) * self.params.damping;

            // A force blow-up would put NaN into positions and corrupt the
            // layout permanently; clamp instead.
            if !node.vel.x.is_finite() || !node.vel.y.is_finite() {
                node.vel = Vec2::default();
            }
            const MAX_SPEED: f32 = 40.0;
            let speed = node.vel.length();
            if speed > MAX_SPEED {
                node.vel.x = node.vel.x / speed * MAX_SPEED;
                node.vel.y = node.vel.y / speed * MAX_SPEED;
            }

            node.pos.x += node.vel.x;
            node.pos.y += node.vel.y;
            energy += node.vel.length();
        }

        self.energy = energy / count as f32;
        self.steps += 1;
        if self.energy < SETTLE_ENERGY || self.steps >= MAX_STEPS {
            self.settled = true;
        }
    }

    /// Runs up to `max_steps`, stopping early once settled.
    pub fn run(&mut self, max_steps: usize) {
        for _ in 0..max_steps {
            if self.settled {
                break;
            }
            self.step();
        }
    }

    /// Pins a node at a position, for dragging.
    pub fn drag(&mut self, node: usize, pos: Vec2) {
        if let Some(n) = self.graph.nodes.get_mut(node) {
            n.pos = pos;
            n.vel = Vec2::default();
            n.pinned = true;
        }
        self.reheat();
    }

    /// Releases a dragged node back into the simulation.
    pub fn release(&mut self, node: usize) {
        if let Some(n) = self.graph.nodes.get_mut(node) {
            n.pinned = false;
        }
        self.reheat();
    }

    /// The node nearest to a point, within `radius`.
    #[must_use]
    pub fn nearest(&self, point: Vec2, radius: f32) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (i, node) in self.graph.nodes.iter().enumerate() {
            let dx = node.pos.x - point.x;
            let dy = node.pos.y - point.y;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist <= radius && best.is_none_or(|(_, b)| dist < b) {
                best = Some((i, dist));
            }
        }
        best.map(|(i, _)| i)
    }
}

// ---------------------------------------------------------------------------
// Barnes-Hut quadtree
// ---------------------------------------------------------------------------

/// Flat-array quadtree over node positions.
///
/// Stored as a `Vec` of cells with index links rather than boxed children: the
/// tree is rebuilt from scratch every simulation step, so allocation cost
/// dominates, and one contiguous buffer is far cheaper than thousands of small
/// allocations.
struct QuadTree {
    cells: Vec<Cell>,
    root: usize,
}

const NO_CELL: usize = usize::MAX;

#[derive(Clone, Copy)]
struct Cell {
    cx: f32,
    cy: f32,
    half: f32,
    mass: f32,
    com_x: f32,
    com_y: f32,
    /// Body held by this cell while it is still a leaf.
    body: usize,
    /// That body's position, kept separately from the center of mass: the
    /// center of mass has already absorbed the incoming body by the time a
    /// leaf splits, so it can't be used to re-place the resident one.
    body_pos: Vec2,
    /// That body's mass, kept for the same reason as `body_pos`.
    body_mass: f32,
    children: [usize; 4],
    leaf: bool,
}

/// How hard a node pushes its neighbours away.
///
/// Weighting by degree is what stops a hub from collecting its whole
/// neighbourhood into a single illegible clump: the more links a note has, the
/// more room it claims, which is also how it reads in Obsidian.
fn mass_of(node: &Node) -> f32 {
    1.0 + node.degree as f32
}

impl Cell {
    fn new(cx: f32, cy: f32, half: f32) -> Self {
        Self {
            cx,
            cy,
            half,
            mass: 0.0,
            com_x: 0.0,
            com_y: 0.0,
            body: NO_CELL,
            body_pos: Vec2::default(),
            body_mass: 0.0,
            children: [NO_CELL; 4],
            leaf: true,
        }
    }
}

/// Cells smaller than this stop subdividing, so coincident nodes terminate.
const MIN_HALF: f32 = 0.05;
/// Belt-and-braces recursion cap alongside [`MIN_HALF`].
const MAX_TREE_DEPTH: usize = 48;

impl QuadTree {
    fn build(nodes: &[Node]) -> Self {
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        for node in nodes {
            min_x = min_x.min(node.pos.x);
            min_y = min_y.min(node.pos.y);
            max_x = max_x.max(node.pos.x);
            max_y = max_y.max(node.pos.y);
        }

        let cx = (min_x + max_x) / 2.0;
        let cy = (min_y + max_y) / 2.0;
        let half = ((max_x - min_x).max(max_y - min_y) / 2.0).max(1.0) * 1.05;

        let mut tree = Self {
            cells: vec![Cell::new(cx, cy, half)],
            root: 0,
        };
        for (i, node) in nodes.iter().enumerate() {
            tree.insert(0, i, node.pos, mass_of(node), 0);
        }
        tree
    }

    fn insert(&mut self, cell: usize, body: usize, pos: Vec2, mass: f32, depth: usize) {
        // Accumulate the center of mass on the way down.
        {
            let c = &mut self.cells[cell];
            let total = c.mass + mass;
            c.com_x = (c.com_x * c.mass + pos.x * mass) / total;
            c.com_y = (c.com_y * c.mass + pos.y * mass) / total;
            c.mass = total;

            if c.half <= MIN_HALF || depth > MAX_TREE_DEPTH {
                // Coincident or near-coincident bodies: aggregate them here
                // rather than subdividing forever.
                return;
            }
        }

        let (leaf, existing, existing_pos, existing_mass) = {
            let c = &self.cells[cell];
            (c.leaf, c.body, c.body_pos, c.body_mass)
        };

        if leaf {
            if existing == NO_CELL {
                let c = &mut self.cells[cell];
                c.body = body;
                c.body_pos = pos;
                c.body_mass = mass;
                return;
            }
            // Split, then push the resident body down before the new one.
            {
                let c = &mut self.cells[cell];
                c.body = NO_CELL;
                c.leaf = false;
            }
            self.subdivide(cell);
            let quadrant = self.quadrant_of(cell, existing_pos);
            self.insert(quadrant, existing, existing_pos, existing_mass, depth + 1);
        }

        let quadrant = self.quadrant_of(cell, pos);
        self.insert(quadrant, body, pos, mass, depth + 1);
    }

    fn subdivide(&mut self, cell: usize) {
        let (cx, cy, half) = {
            let c = &self.cells[cell];
            (c.cx, c.cy, c.half / 2.0)
        };
        for (i, (sx, sy)) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)]
            .into_iter()
            .enumerate()
        {
            self.cells
                .push(Cell::new(cx + sx * half, cy + sy * half, half));
            self.cells[cell].children[i] = self.cells.len() - 1;
        }
    }

    fn quadrant_of(&self, cell: usize, pos: Vec2) -> usize {
        let c = &self.cells[cell];
        let index = usize::from(pos.x >= c.cx) + 2 * usize::from(pos.y >= c.cy);
        c.children[index]
    }

    /// Repulsive force on `body` from every other node.
    fn repulsion(&self, nodes: &[Node], body: usize, strength: f32, theta: f32) -> Vec2 {
        let pos = nodes[body].pos;
        let mut force = Vec2::default();
        let mut stack = vec![self.root];

        while let Some(index) = stack.pop() {
            if index == NO_CELL {
                continue;
            }
            let cell = &self.cells[index];
            if cell.mass == 0.0 {
                continue;
            }

            let dx = pos.x - cell.com_x;
            let dy = pos.y - cell.com_y;
            let dist_sq = dx * dx + dy * dy;

            // Two nodes at the same spot would divide by zero; nudge them apart
            // deterministically using the body index so the result is stable.
            if dist_sq < 1e-6 {
                let angle = body as f32 * 0.7;
                force.x += angle.cos() * strength * 0.01;
                force.y += angle.sin() * strength * 0.01;
                continue;
            }

            let dist = dist_sq.sqrt();
            let is_single_body = cell.leaf && cell.body != NO_CELL;

            if is_single_body || (cell.half * 2.0 / dist) < theta {
                if is_single_body && cell.body == body {
                    continue;
                }
                let magnitude = strength * cell.mass / dist_sq;
                force.x += dx / dist * magnitude;
                force.y += dy / dist * magnitude;
            } else {
                stack.extend(cell.children.iter().copied().filter(|&c| c != NO_CELL));
            }
        }

        force
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempVault;

    fn linked_vault() -> TempVault {
        let vault = TempVault::new("graph");
        vault.write("A.md", "#topic\nlinks to [[B]] and [[Ghost]]\n");
        vault.write("B.md", "links back to [[A]]\n");
        vault.write("Lonely.md", "no links\n");
        vault
    }

    #[test]
    fn builds_nodes_and_edges_from_links() {
        let vault = linked_vault();
        let index = vault.index();
        let graph = Graph::build(&index, &GraphOptions::default());

        assert_eq!(
            graph.nodes.len(),
            4,
            "three notes plus one unresolved target"
        );
        assert_eq!(
            graph.edges.len(),
            2,
            "A-B is one edge despite being mutual, plus A-Ghost"
        );
    }

    #[test]
    fn unresolved_nodes_can_be_hidden() {
        let vault = linked_vault();
        let index = vault.index();
        let graph = Graph::build(
            &index,
            &GraphOptions {
                show_unresolved: false,
                ..Default::default()
            },
        );
        assert!(!graph.nodes.iter().any(|n| n.kind == NodeKind::Unresolved));
    }

    #[test]
    fn orphans_can_be_hidden() {
        let vault = linked_vault();
        let index = vault.index();
        let graph = Graph::build(
            &index,
            &GraphOptions {
                show_orphans: false,
                ..Default::default()
            },
        );
        assert!(
            !graph.nodes.iter().any(|n| n.label == "Lonely"),
            "an unlinked note is an orphan"
        );
        // Edges must still be valid after the reindexing that removal implies.
        for edge in &graph.edges {
            assert!(edge.from < graph.nodes.len() && edge.to < graph.nodes.len());
        }
    }

    #[test]
    fn tag_nodes_are_opt_in() {
        let vault = linked_vault();
        let index = vault.index();

        let without = Graph::build(&index, &GraphOptions::default());
        assert!(!without.nodes.iter().any(|n| n.kind == NodeKind::Tag));

        let with = Graph::build(
            &index,
            &GraphOptions {
                show_tags: true,
                ..Default::default()
            },
        );
        assert!(with.nodes.iter().any(|n| n.label == "#topic"));
    }

    #[test]
    fn tag_filter_restricts_to_matching_notes() {
        let vault = linked_vault();
        let index = vault.index();
        let graph = Graph::build(
            &index,
            &GraphOptions {
                tag_filter: Some("topic".into()),
                show_unresolved: false,
                ..Default::default()
            },
        );
        let labels: Vec<_> = graph.nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(labels, vec!["A"]);
    }

    #[test]
    fn degree_counts_edges() {
        let vault = linked_vault();
        let index = vault.index();
        let graph = Graph::build(&index, &GraphOptions::default());

        let a = graph.nodes.iter().find(|n| n.label == "A").unwrap();
        assert_eq!(a.degree, 2, "A links to B and to Ghost");
    }

    #[test]
    fn neighborhood_respects_depth() {
        let vault = TempVault::new("neighborhood");
        vault.write("A.md", "[[B]]\n");
        vault.write("B.md", "[[C]]\n");
        vault.write("C.md", "end\n");

        let index = vault.index();
        let graph = Graph::build(&index, &GraphOptions::default());
        let a = graph
            .node_of_note(index.id_of_rel("A.md").unwrap())
            .unwrap();

        assert_eq!(graph.neighborhood(a, 0).len(), 1);
        assert_eq!(graph.neighborhood(a, 1).len(), 2);
        assert_eq!(graph.neighborhood(a, 2).len(), 3);
    }

    #[test]
    fn a_subgraph_stands_on_its_own() {
        let vault = TempVault::new("subgraph");
        vault.write("A.md", "[[B]]\n");
        vault.write("B.md", "[[C]]\n");
        vault.write("C.md", "end\n");

        let index = vault.index();
        let graph = Graph::build(&index, &GraphOptions::default());
        let a = graph
            .node_of_note(index.id_of_rel("A.md").unwrap())
            .unwrap();

        let local = graph.subgraph(&graph.neighborhood(a, 1));

        let labels: Vec<&str> = local.nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(labels, vec!["A", "B"], "one hop from A reaches B, not C");
        assert_eq!(
            local.edges.len(),
            1,
            "B's link to C has nowhere to land and must be dropped, not left dangling"
        );
        // Indices are remapped, or an edge points past the end of the node list.
        for edge in &local.edges {
            assert!(edge.from < local.nodes.len() && edge.to < local.nodes.len());
        }
        // Degree is recounted over the kept edges: inside this view B is a leaf.
        assert_eq!(local.nodes[1].degree, 1, "B keeps only its link to A");
        // Positions are re-seeded, or the layout starts with every node stacked
        // on the coordinates it held in the full graph.
        assert_ne!(local.nodes[0].pos, local.nodes[1].pos);
    }

    #[test]
    fn a_subgraph_ignores_indices_that_do_not_exist() {
        let vault = linked_vault();
        let index = vault.index();
        let graph = Graph::build(&index, &GraphOptions::default());

        let local = graph.subgraph(&[0, 0, usize::MAX, 999]);
        assert_eq!(local.nodes.len(), 1, "duplicates and strays are skipped");
    }

    #[test]
    fn layout_is_deterministic() {
        let vault = linked_vault();
        let index = vault.index();

        let mut first = Simulation::new(Graph::build(&index, &GraphOptions::default()));
        let mut second = Simulation::new(Graph::build(&index, &GraphOptions::default()));
        first.run(200);
        second.run(200);

        for (a, b) in first.graph.nodes.iter().zip(second.graph.nodes.iter()) {
            assert!((a.pos.x - b.pos.x).abs() < 1e-4);
            assert!((a.pos.y - b.pos.y).abs() < 1e-4);
        }
    }

    #[test]
    fn simulation_settles_and_stops() {
        let vault = linked_vault();
        let index = vault.index();
        let mut sim = Simulation::new(Graph::build(&index, &GraphOptions::default()));

        sim.run(MAX_STEPS);
        assert!(sim.is_settled(), "energy was {}", sim.energy());

        // Once settled, stepping must not move anything — that's what makes an
        // idle graph free.
        let before: Vec<Vec2> = sim.graph.nodes.iter().map(|n| n.pos).collect();
        sim.step();
        let after: Vec<Vec2> = sim.graph.nodes.iter().map(|n| n.pos).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn positions_stay_finite_under_stress() {
        let vault = TempVault::new("stress");
        // A hub-and-spoke graph puts many nodes at similar positions, which is
        // where a naive force calculation blows up.
        let mut hub = String::new();
        for i in 0..60 {
            hub.push_str(&format!("[[N{i}]]\n"));
            vault.write(&format!("N{i}.md"), "[[Hub]]\n");
        }
        vault.write("Hub.md", &hub);

        let index = vault.index();
        let mut sim = Simulation::new(Graph::build(&index, &GraphOptions::default()));
        sim.run(500);

        for node in &sim.graph.nodes {
            assert!(
                node.pos.x.is_finite() && node.pos.y.is_finite(),
                "node {} went non-finite",
                node.label
            );
        }
    }

    #[test]
    fn linked_nodes_end_up_closer_than_unlinked_ones() {
        let vault = TempVault::new("layout-quality");
        vault.write("A.md", "[[B]]\n");
        vault.write("B.md", "b\n");
        vault.write("Far.md", "unrelated\n");

        let index = vault.index();
        let mut sim = Simulation::new(Graph::build(&index, &GraphOptions::default()));
        sim.run(1500);

        let pos = |label: &str| {
            sim.graph
                .nodes
                .iter()
                .find(|n| n.label == label)
                .unwrap()
                .pos
        };
        let dist = |a: Vec2, b: Vec2| ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();

        let linked = dist(pos("A"), pos("B"));
        let unlinked = dist(pos("A"), pos("Far"));
        assert!(
            linked < unlinked,
            "linked {linked} should be closer than unlinked {unlinked}"
        );
    }

    /// How well spread a settled layout is.
    ///
    /// The closest pair of nodes, measured against the spacing an even scatter
    /// over the same bounding box would give. Below 1 the layout is clumpier
    /// than an even scatter; this is the number that decides whether the graph
    /// reads as distinct notes or as one smear, because the view auto-fits to
    /// the bounding box and so only relative spacing survives to the screen.
    fn spread(sim: &Simulation) -> f32 {
        let nodes = &sim.graph.nodes;
        let mut closest = f32::MAX;
        for (i, a) in nodes.iter().enumerate() {
            for b in &nodes[i + 1..] {
                closest = closest.min((a.pos.x - b.pos.x).hypot(a.pos.y - b.pos.y));
            }
        }
        let (min_x, min_y, max_x, max_y) = sim.graph.bounds();
        let diameter = (max_x - min_x).max(max_y - min_y);
        closest / (diameter / (nodes.len() as f32).sqrt())
    }

    #[test]
    fn the_layout_spreads_nodes_out() {
        // The shape that used to collapse: a hub with many spokes, a couple of
        // interlinked notes off to one side, an orphan, and an unresolved link.
        let vault = TempVault::new("spread");
        let mut hub = String::new();
        for i in 0..8 {
            hub.push_str(&format!("[[S{i}]]\n"));
            vault.write(&format!("S{i}.md"), "[[Hub]]\n");
        }
        hub.push_str("[[Ghost]]\n");
        vault.write("Hub.md", &hub);
        vault.write("A.md", "[[B]] and [[Hub]]\n");
        vault.write("B.md", "[[A]]\n");
        vault.write("Orphan.md", "no links\n");

        let index = vault.index();
        let mut sim = Simulation::new(Graph::build(&index, &GraphOptions::default()));
        sim.run(MAX_STEPS);

        let spread = spread(&sim);
        assert!(
            spread > 0.5,
            "nodes are bunched too tightly to tell apart: spread {spread}"
        );
    }

    #[test]
    fn dragging_pins_a_node_in_place() {
        let vault = linked_vault();
        let index = vault.index();
        let mut sim = Simulation::new(Graph::build(&index, &GraphOptions::default()));

        sim.drag(0, Vec2::new(100.0, 100.0));
        sim.run(50);
        assert_eq!(sim.graph.nodes[0].pos, Vec2::new(100.0, 100.0));

        sim.release(0);
        sim.run(50);
        assert_ne!(sim.graph.nodes[0].pos, Vec2::new(100.0, 100.0));
    }

    #[test]
    fn nearest_finds_nodes_within_radius() {
        let vault = linked_vault();
        let index = vault.index();
        let mut sim = Simulation::new(Graph::build(&index, &GraphOptions::default()));
        sim.drag(0, Vec2::new(10.0, 10.0));

        assert_eq!(sim.nearest(Vec2::new(11.0, 10.0), 5.0), Some(0));
        assert_eq!(sim.nearest(Vec2::new(500.0, 500.0), 5.0), None);
    }

    #[test]
    fn empty_graph_is_handled() {
        let vault = TempVault::new("empty-graph");
        let index = vault.index();
        let mut sim = Simulation::new(Graph::build(&index, &GraphOptions::default()));
        sim.step();
        assert!(sim.graph.is_empty());
        assert_eq!(sim.graph.bounds(), (-1.0, -1.0, 1.0, 1.0));
    }
}
