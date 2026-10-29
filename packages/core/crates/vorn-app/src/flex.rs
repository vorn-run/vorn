//! The Flexible grid: the cards as a split tree, each split two children
//! side by side or stacked, its divider dragged to resize them.
//!
//! A leaf is a session's id or a popped-out card's. The tree is kept per
//! workspace in the viewer's `gridLayout` setting, in the shape a saved
//! layout's nodes take: `{"id"}` for a leaf, `{"split", "ratio",
//! "children"}` for a split.

use serde_json::{json, Map, Value};

/// How a split lays out its two children.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    /// Side by side: the divider is vertical (a layout's `horizontal`).
    Row,
    /// Stacked: the divider is horizontal (a layout's `vertical`).
    Col,
}

impl Dir {
    fn key(self) -> &'static str {
        match self {
            Dir::Row => "horizontal",
            Dir::Col => "vertical",
        }
    }

    fn parse(key: &str) -> Option<Dir> {
        match key {
            "horizontal" => Some(Dir::Row),
            "vertical" => Some(Dir::Col),
            _ => None,
        }
    }
}

/// An arrow key's direction, for moving between panes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Up,
    Down,
}

impl Side {
    /// The splits whose divider this direction moves.
    pub fn dir(self) -> Dir {
        match self {
            Side::Left | Side::Right => Dir::Row,
            Side::Up | Side::Down => Dir::Col,
        }
    }

    /// Whether the direction grows the first child.
    fn forward(self) -> bool {
        matches!(self, Side::Right | Side::Down)
    }
}

/// A box in the grid's area, in the screen's units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    fn extent(&self, dir: Dir) -> f32 {
        match dir {
            Dir::Row => self.w,
            Dir::Col => self.h,
        }
    }

    /// The two parts of a split at `ratio`.
    fn cut(&self, dir: Dir, ratio: f32) -> (Rect, Rect) {
        match dir {
            Dir::Row => {
                let a = self.w * ratio;
                (
                    Rect::new(self.x, self.y, a, self.h),
                    Rect::new(self.x + a, self.y, self.w - a, self.h),
                )
            }
            Dir::Col => {
                let a = self.h * ratio;
                (
                    Rect::new(self.x, self.y, self.w, a),
                    Rect::new(self.x, self.y + a, self.w, self.h - a),
                )
            }
        }
    }
}

/// The smallest a pane may be, as (width, height).
pub type Min = (f32, f32);

/// Where a split sits: 1 for the root, then `2p` for a split's first child
/// and `2p + 1` for its second. Stable while the splits above it are.
pub type Path = u64;

/// The root's path.
pub const ROOT: Path = 1;

/// The deepest a path can name; a tree this deep never comes from people.
const MAX_DEPTH: u32 = 62;

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Leaf(String),
    Split {
        dir: Dir,
        /// The first child's share of the split's extent.
        ratio: f32,
        a: Box<Node>,
        b: Box<Node>,
    },
}

impl Node {
    pub fn leaf(id: &str) -> Node {
        Node::Leaf(id.to_owned())
    }

    pub fn split(dir: Dir, ratio: f32, a: Node, b: Node) -> Node {
        Node::Split {
            dir,
            ratio,
            a: Box::new(a),
            b: Box::new(b),
        }
    }

    /// The leaves' ids, first to last.
    pub fn leaves(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.walk(&mut |id| out.push(id));
        out
    }

    fn walk<'a>(&'a self, f: &mut impl FnMut(&'a str)) {
        match self {
            Node::Leaf(id) => f(id),
            Node::Split { a, b, .. } => {
                a.walk(f);
                b.walk(f);
            }
        }
    }

    pub fn contains(&self, id: &str) -> bool {
        match self {
            Node::Leaf(l) => l == id,
            Node::Split { a, b, .. } => a.contains(id) || b.contains(id),
        }
    }

    /// The split at `path`.
    pub fn at(&self, path: Path) -> Option<&Node> {
        let depth = 63 - path.leading_zeros();
        let mut node = self;
        for bit in (0..depth).rev() {
            node = match node {
                Node::Split { a, b, .. } => {
                    if path >> bit & 1 == 0 {
                        a
                    } else {
                        b
                    }
                }
                Node::Leaf(_) => return None,
            };
        }
        Some(node)
    }

    fn at_mut(&mut self, path: Path) -> Option<&mut Node> {
        let depth = 63 - path.leading_zeros();
        let mut node = self;
        for bit in (0..depth).rev() {
            node = match node {
                Node::Split { a, b, .. } => {
                    if path >> bit & 1 == 0 {
                        a
                    } else {
                        b
                    }
                }
                Node::Leaf(_) => return None,
            };
        }
        Some(node)
    }

    /// Every leaf's box when the tree fills `area`.
    pub fn rects(&self, area: Rect) -> Vec<(&str, Rect)> {
        let mut out = Vec::new();
        self.rects_into(area, &mut out);
        out
    }

    fn rects_into<'a>(&'a self, area: Rect, out: &mut Vec<(&'a str, Rect)>) {
        match self {
            Node::Leaf(id) => out.push((id, area)),
            Node::Split { dir, ratio, a, b } => {
                let (ra, rb) = area.cut(*dir, *ratio);
                a.rects_into(ra, out);
                b.rects_into(rb, out);
            }
        }
    }

    /// The box of the node at `path` when the tree fills `area`.
    fn rect_at(&self, path: Path, area: Rect) -> Option<Rect> {
        let depth = 63 - path.leading_zeros();
        let (mut node, mut rect) = (self, area);
        for bit in (0..depth).rev() {
            let Node::Split { dir, ratio, a, b } = node else {
                return None;
            };
            let (ra, rb) = rect.cut(*dir, *ratio);
            (node, rect) = if path >> bit & 1 == 0 {
                (a.as_ref(), ra)
            } else {
                (b.as_ref(), rb)
            };
        }
        Some(rect)
    }

    /// The leaf `id`'s path.
    pub fn path_of(&self, id: &str) -> Option<Path> {
        fn find(node: &Node, id: &str, path: Path) -> Option<Path> {
            match node {
                Node::Leaf(l) => (l == id).then_some(path),
                Node::Split { a, b, .. } if path.leading_zeros() > 64 - MAX_DEPTH => {
                    find(a, id, path << 1).or_else(|| find(b, id, path << 1 | 1))
                }
                Node::Split { .. } => None,
            }
        }
        find(self, id, ROOT)
    }

    /// Every split's path, parents first.
    pub fn splits(&self) -> Vec<Path> {
        fn walk(node: &Node, path: Path, out: &mut Vec<Path>) {
            if let Node::Split { a, b, .. } = node {
                out.push(path);
                if path.leading_zeros() > 64 - MAX_DEPTH {
                    walk(a, path << 1, out);
                    walk(b, path << 1 | 1, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(self, ROOT, &mut out);
        out
    }

    /// The smallest the node can be along `dir`: its panes' minimums added
    /// up along the axis, the largest of them across it.
    pub fn min_extent(&self, dir: Dir, min: Min) -> f32 {
        match self {
            Node::Leaf(_) => match dir {
                Dir::Row => min.0,
                Dir::Col => min.1,
            },
            Node::Split { dir: d, a, b, .. } if *d == dir => {
                a.min_extent(dir, min) + b.min_extent(dir, min)
            }
            Node::Split { a, b, .. } => a.min_extent(dir, min).max(b.min_extent(dir, min)),
        }
    }

    /// How many panes the node lines up along `dir`, for equal sizes.
    fn weight(&self, dir: Dir) -> f32 {
        match self {
            Node::Leaf(_) => 1.0,
            Node::Split { dir: d, a, b, .. } if *d == dir => a.weight(dir) + b.weight(dir),
            Node::Split { a, b, .. } => a.weight(dir).max(b.weight(dir)),
        }
    }

    /// Every split's ratio set so the panes along it come out the same size.
    pub fn equalize(&mut self) {
        if let Node::Split { dir, ratio, a, b } = self {
            a.equalize();
            b.equalize();
            let (wa, wb) = (a.weight(*dir), b.weight(*dir));
            *ratio = wa / (wa + wb);
        }
    }

    /// The ratio the split at `path` may take in `area`: each side keeps
    /// its panes' minimums. A split too small for both is halved.
    pub fn clamp(&self, path: Path, ratio: f32, area: Rect, min: Min) -> f32 {
        let (Some(Node::Split { dir, a, b, .. }), Some(rect)) =
            (self.at(path), self.rect_at(path, area))
        else {
            return ratio;
        };
        let extent = rect.extent(*dir);
        if extent <= 0.0 {
            return ratio;
        }
        let lo = a.min_extent(*dir, min) / extent;
        let hi = 1.0 - b.min_extent(*dir, min) / extent;
        if lo > hi {
            return 0.5;
        }
        ratio.clamp(lo, hi)
    }

    /// Sets the split at `path` to `ratio`, kept within its panes' minimums.
    /// Whether it moved.
    pub fn resize(&mut self, path: Path, ratio: f32, area: Rect, min: Min) -> bool {
        let ratio = self.clamp(path, ratio, area, min);
        match self.at_mut(path) {
            Some(Node::Split { ratio: r, .. }) if (*r - ratio).abs() > f32::EPSILON => {
                *r = ratio;
                true
            }
            _ => false,
        }
    }

    /// Moves the divider nearest the leaf `id` along `side` by `step` of
    /// its split. Whether it moved.
    pub fn nudge(&mut self, id: &str, side: Side, step: f32, area: Rect, min: Min) -> bool {
        let Some(mut path) = self.path_of(id) else {
            return false;
        };
        while path > ROOT {
            path >>= 1;
            if let Some(Node::Split { dir, ratio, .. }) = self.at(path) {
                if *dir == side.dir() {
                    let to = if side.forward() {
                        ratio + step
                    } else {
                        ratio - step
                    };
                    return self.resize(path, to, area, min);
                }
            }
        }
        false
    }

    /// Takes the leaf `id` out; its sibling takes the parent's place.
    /// None when it was the last leaf.
    pub fn remove(self, id: &str) -> Option<Node> {
        match self {
            Node::Leaf(l) if l == id => None,
            leaf @ Node::Leaf(_) => Some(leaf),
            Node::Split { dir, ratio, a, b } => match (a.remove(id), b.remove(id)) {
                (Some(a), Some(b)) => Some(Node::split(dir, ratio, a, b)),
                (one, other) => one.or(other),
            },
        }
    }

    /// Keeps only the leaves `keep` holds.
    pub fn retain(self, keep: &impl Fn(&str) -> bool) -> Option<Node> {
        match self {
            Node::Leaf(id) => keep(&id).then_some(Node::Leaf(id)),
            Node::Split { dir, ratio, a, b } => match (a.retain(keep), b.retain(keep)) {
                (Some(a), Some(b)) => Some(Node::split(dir, ratio, a, b)),
                (one, other) => one.or(other),
            },
        }
    }

    /// Splits the leaf `at` in two along its longer side, `id` after it.
    /// Without `at` (or when it is gone) the largest pane is split.
    pub fn insert(&mut self, id: &str, at: Option<&str>, area: Rect) {
        let rects = self.rects(area);
        let pick = at
            .and_then(|at| rects.iter().find(|(l, _)| *l == at))
            .or_else(|| {
                rects
                    .iter()
                    .max_by(|x, y| (x.1.w * x.1.h).total_cmp(&(y.1.w * y.1.h)))
            })
            .map(|(l, r)| ((*l).to_owned(), *r));
        let Some((target, rect)) = pick else {
            return;
        };
        let dir = if rect.w >= rect.h { Dir::Row } else { Dir::Col };
        self.replace_leaf(&target, &mut |old| {
            Node::split(dir, 0.5, Node::Leaf(old), Node::leaf(id))
        });
    }

    /// Splits the leaf `at` in two along `dir`, `id` after it (right of it
    /// or under it) at half its room. Whether `at` was there.
    pub fn insert_beside(&mut self, id: &str, at: &str, dir: Dir) -> bool {
        self.replace_leaf(at, &mut |old| {
            Node::split(dir, 0.5, Node::Leaf(old), Node::leaf(id))
        })
    }

    fn replace_leaf(&mut self, target: &str, f: &mut impl FnMut(String) -> Node) -> bool {
        match self {
            Node::Leaf(l) if l == target => {
                let old = std::mem::take(l);
                *self = f(old);
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.replace_leaf(target, f) || b.replace_leaf(target, f),
        }
    }

    /// The tree as the automatic grid lays `ids` out: rows of `cols`.
    pub fn from_grid(ids: &[&str], cols: usize) -> Option<Node> {
        let cols = cols.max(1);
        let rows: Vec<Node> = ids
            .chunks(cols)
            .filter_map(|row| chain(Dir::Row, row.iter().map(|id| Node::leaf(id)).collect()))
            .collect();
        let mut tree = chain(Dir::Col, rows)?;
        tree.equalize();
        Some(tree)
    }

    /// The tree as JSON: `{"id"}` for a leaf, `{"split", "ratio",
    /// "children"}` for a split.
    pub fn to_json(&self) -> Value {
        match self {
            Node::Leaf(id) => json!({ "id": id }),
            Node::Split { dir, ratio, a, b } => json!({
                "split": dir.key(),
                "ratio": round(*ratio),
                "children": [a.to_json(), b.to_json()],
            }),
        }
    }

    /// Reads [`Node::to_json`]'s form. A split of more than two children
    /// (as a saved layout writes) nests into twos at the same sizes: the
    /// first child takes `ratio`, the rest share what is left equally.
    pub fn from_json(value: &Value) -> Option<Node> {
        if let Some(id) = value.get("id").and_then(Value::as_str) {
            return Some(Node::leaf(id));
        }
        let dir = Dir::parse(value.get("split")?.as_str()?)?;
        let children: Vec<Node> = value
            .get("children")?
            .as_array()?
            .iter()
            .filter_map(Node::from_json)
            .collect();
        let n = children.len();
        let ratio = value
            .get("ratio")
            .and_then(Value::as_f64)
            .map(|r| r as f32)
            .filter(|r| r.is_finite() && *r > 0.0 && *r < 1.0);
        let mut it = children.into_iter().rev();
        let mut node = it.next()?;
        // Built from the back: the split before the last k children gives
        // its first child 1/(k+1) of what it holds, the root `ratio`.
        for (k, child) in it.enumerate() {
            let first = k + 2 == n;
            let r = match ratio {
                Some(r) if first => r,
                _ => 1.0 / (k + 2) as f32,
            };
            node = Node::split(dir, r, child, node);
        }
        Some(node)
    }
}

/// `nodes` as nested splits along `dir`, each first child before the rest.
pub fn chain(dir: Dir, nodes: Vec<Node>) -> Option<Node> {
    let mut it = nodes.into_iter().rev();
    let mut node = it.next()?;
    for (k, child) in it.enumerate() {
        node = Node::split(dir, 1.0 / (k + 2) as f32, child, node);
    }
    Some(node)
}

fn round(r: f32) -> f64 {
    (f64::from(r) * 10_000.0).round() / 10_000.0
}

/// The pane beside `from` toward `side`: of those past its edge and
/// overlapping it across, the nearest, then the most overlapped.
pub fn neighbour<'a>(rects: &[(&'a str, Rect)], from: &str, side: Side) -> Option<&'a str> {
    let (_, r) = rects.iter().find(|(id, _)| *id == from)?;
    const EPS: f32 = 0.5;
    let overlap = |a0: f32, a1: f32, b0: f32, b1: f32| (a1.min(b1) - a0.max(b0)).max(0.0);
    rects
        .iter()
        .filter(|(id, _)| *id != from)
        .filter_map(|(id, o)| {
            let (gap, over) = match side {
                Side::Right => (o.x - (r.x + r.w), overlap(r.y, r.y + r.h, o.y, o.y + o.h)),
                Side::Left => (r.x - (o.x + o.w), overlap(r.y, r.y + r.h, o.y, o.y + o.h)),
                Side::Down => (o.y - (r.y + r.h), overlap(r.x, r.x + r.w, o.x, o.x + o.w)),
                Side::Up => (r.y - (o.y + o.h), overlap(r.x, r.x + r.w, o.x, o.x + o.w)),
            };
            (gap > -EPS && over > EPS).then_some((*id, gap, over))
        })
        .min_by(|x, y| x.1.total_cmp(&y.1).then(y.2.total_cmp(&x.2)))
        .map(|(id, _, _)| id)
}

/// One workspace's grid: how many columns, or the flexible tree.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Grid {
    /// 0 for automatic, 2 to 4 fixed; ignored while `flexible`.
    pub columns: i32,
    pub flexible: bool,
    pub tree: Option<Node>,
}

impl Grid {
    /// The columns the screen's menu knows: -1 for flexible.
    pub fn mode(&self) -> i32 {
        if self.flexible {
            -1
        } else {
            self.columns
        }
    }

    pub fn from_json(value: &Value) -> Grid {
        Grid {
            columns: value
                .get("columns")
                .and_then(Value::as_i64)
                .and_then(|n| i32::try_from(n).ok())
                .filter(|n| *n >= 0)
                .unwrap_or(0),
            flexible: value.get("view").and_then(Value::as_str) == Some("flexible"),
            tree: value.get("tree").and_then(Node::from_json),
        }
    }

    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert(
            "view".into(),
            json!(if self.flexible { "flexible" } else { "grid" }),
        );
        out.insert("columns".into(), json!(self.columns));
        if let Some(tree) = &self.tree {
            out.insert("tree".into(), tree.to_json());
        }
        Value::Object(out)
    }
}

/// Every workspace's grid, from the viewer's `gridLayout` setting.
pub fn grids_from_json(value: Option<&Value>) -> std::collections::BTreeMap<String, Grid> {
    value
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(ws, g)| (ws.clone(), Grid::from_json(g)))
                .collect()
        })
        .unwrap_or_default()
}

pub fn grids_to_json(grids: &std::collections::BTreeMap<String, Grid>) -> Value {
    Value::Object(
        grids
            .iter()
            .map(|(ws, g)| (ws.clone(), g.to_json()))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 1200.0,
        h: 800.0,
    };
    const MIN: Min = (240.0, 160.0);

    fn ids(n: &Node) -> Vec<String> {
        n.leaves().into_iter().map(str::to_owned).collect()
    }

    #[test]
    fn insert_splits_the_anchor_along_its_longer_side() {
        let mut t = Node::leaf("a");
        t.insert("b", Some("a"), AREA);
        assert_eq!(
            t,
            Node::split(Dir::Row, 0.5, Node::leaf("a"), Node::leaf("b"))
        );
        // "b" is now 600×800: taller than wide, so it stacks.
        t.insert("c", Some("b"), AREA);
        let r = t.rects(AREA);
        assert_eq!(r[1], ("b", Rect::new(600.0, 0.0, 600.0, 400.0)));
        assert_eq!(r[2], ("c", Rect::new(600.0, 400.0, 600.0, 400.0)));
    }

    #[test]
    fn a_split_puts_the_new_pane_right_of_or_under_its_card_only() {
        let mut t = Node::split(Dir::Row, 0.3, Node::leaf("a"), Node::leaf("b"));
        assert!(t.insert_beside("c", "b", Dir::Row));
        assert_eq!(
            t,
            Node::split(
                Dir::Row,
                0.3,
                Node::leaf("a"),
                Node::split(Dir::Row, 0.5, Node::leaf("b"), Node::leaf("c")),
            ),
            "beside, at half the card's room, the others untouched"
        );
        assert!(t.insert_beside("d", "a", Dir::Col));
        let r = t.rects(AREA);
        assert_eq!(r[0], ("a", Rect::new(0.0, 0.0, 360.0, 400.0)));
        assert_eq!(r[1], ("d", Rect::new(0.0, 400.0, 360.0, 400.0)));
        assert!(!t.insert_beside("e", "gone", Dir::Row));
        assert_eq!(ids(&t), ["a", "d", "b", "c"]);
    }

    #[test]
    fn insert_without_an_anchor_splits_the_largest_pane() {
        let mut t = Node::split(Dir::Row, 0.7, Node::leaf("a"), Node::leaf("b"));
        t.insert("c", Some("gone"), AREA);
        assert_eq!(ids(&t), ["a", "c", "b"]);
    }

    #[test]
    fn remove_collapses_the_parent_into_the_sibling() {
        let t = Node::split(
            Dir::Row,
            0.3,
            Node::leaf("a"),
            Node::split(Dir::Col, 0.6, Node::leaf("b"), Node::leaf("c")),
        );
        let t = t.remove("b").unwrap();
        assert_eq!(
            t,
            Node::split(Dir::Row, 0.3, Node::leaf("a"), Node::leaf("c"))
        );
        let t = t.remove("a").unwrap();
        assert_eq!(t, Node::leaf("c"));
        assert_eq!(t.remove("c"), None);
    }

    #[test]
    fn resize_keeps_each_side_above_its_minimum() {
        let mut t = Node::split(
            Dir::Row,
            0.5,
            Node::leaf("a"),
            Node::split(Dir::Row, 0.5, Node::leaf("b"), Node::leaf("c")),
        );
        // The right side holds two panes side by side: 480 of 1200 at least.
        assert!(t.resize(ROOT, 0.9, AREA, MIN));
        let Node::Split { ratio, .. } = t else {
            unreachable!()
        };
        assert!((ratio - 0.6).abs() < 1e-6);
        assert!(t.resize(ROOT, 0.01, AREA, MIN));
        assert!((t.clamp(ROOT, 0.0, AREA, MIN) - 0.2).abs() < 1e-6);
        // The nested split measures against its own box (960 wide here).
        assert!(t.resize(3, 0.95, AREA, MIN));
        let r = t.rects(AREA);
        assert!((r[2].1.w - 240.0).abs() < 0.01, "{r:?}");
    }

    #[test]
    fn equalize_gives_every_pane_along_an_axis_the_same_share() {
        let mut t = chain(
            Dir::Row,
            vec![Node::leaf("a"), Node::leaf("b"), Node::leaf("c")],
        )
        .unwrap();
        t.resize(ROOT, 0.8, AREA, (0.0, 0.0));
        t.equalize();
        for (_, r) in t.rects(AREA) {
            assert!((r.w - 400.0).abs() < 0.01);
        }
    }

    #[test]
    fn neighbours_follow_the_arrows() {
        // a | b
        //   | c
        let t = Node::split(
            Dir::Row,
            0.5,
            Node::leaf("a"),
            Node::split(Dir::Col, 0.5, Node::leaf("b"), Node::leaf("c")),
        );
        let r = t.rects(AREA);
        assert_eq!(neighbour(&r, "a", Side::Right), Some("b"));
        assert_eq!(neighbour(&r, "c", Side::Left), Some("a"));
        assert_eq!(neighbour(&r, "b", Side::Down), Some("c"));
        assert_eq!(neighbour(&r, "c", Side::Up), Some("b"));
        assert_eq!(neighbour(&r, "a", Side::Up), None);
    }

    #[test]
    fn nudge_moves_the_nearest_divider_on_that_axis() {
        let mut t = Node::split(
            Dir::Row,
            0.5,
            Node::leaf("a"),
            Node::split(Dir::Col, 0.5, Node::leaf("b"), Node::leaf("c")),
        );
        assert!(t.nudge("c", Side::Left, 0.05, AREA, MIN));
        let Node::Split { ratio, .. } = &t else {
            unreachable!()
        };
        assert!((ratio - 0.45).abs() < 1e-6);
        assert!(t.nudge("c", Side::Up, 0.05, AREA, MIN));
        assert!((t.rects(AREA)[1].1.h - 360.0).abs() < 0.01);
    }

    #[test]
    fn json_round_trips_and_reads_wider_splits() {
        let t = Node::split(
            Dir::Col,
            0.25,
            Node::leaf("a"),
            Node::split(Dir::Row, 0.5, Node::leaf("b"), Node::leaf("c")),
        );
        assert_eq!(Node::from_json(&t.to_json()), Some(t));

        let wide = json!({"split": "horizontal", "ratio": 0.4, "children": [
            {"id": "a"}, {"id": "b"}, {"id": "c"}
        ]});
        let t = Node::from_json(&wide).unwrap();
        let w: Vec<f32> = t.rects(AREA).iter().map(|(_, r)| r.w).collect();
        assert!((w[0] - 480.0).abs() < 0.01 && (w[1] - 360.0).abs() < 0.01);
        assert!((w[2] - 360.0).abs() < 0.01);
    }

    #[test]
    fn from_grid_matches_the_automatic_rows() {
        let t = Node::from_grid(&["a", "b", "c", "d", "e"], 3).unwrap();
        let r = t.rects(AREA);
        assert_eq!(r.len(), 5);
        assert!((r[0].1.w - 400.0).abs() < 0.01);
        assert!((r[3].1.y - 400.0).abs() < 0.01 && (r[3].1.w - 600.0).abs() < 0.01);
    }

    #[test]
    fn grids_keep_each_workspace() {
        let mut grids = std::collections::BTreeMap::new();
        grids.insert(
            "personal".to_owned(),
            Grid {
                columns: 3,
                flexible: true,
                tree: Node::from_grid(&["a", "b"], 2),
            },
        );
        let back = grids_from_json(Some(&grids_to_json(&grids)));
        assert_eq!(back, grids);
        assert_eq!(back["personal"].mode(), -1);
    }
}
