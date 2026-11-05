//! Box layout for markup: a small flexbox subset.
//!
//! - `row` / `column`: children along the main axis, `gap` between them. `grow` (or a `fill`
//!   size on the main axis) shares the free space. Without growing children, `justify`
//!   (start | center | end | space-between) places them. `align` (start | center | end | stretch)
//!   places each child on the cross axis.
//! - `stack`: children on top of each other, centered unless `align` / `justify` say otherwise.
//! - `width` / `height`: a number in DIPs, `fill`, or absent (natural size).
//! - `padding`: "8", "8 16" (vertical horizontal), or "t r b l".

use crate::package::Node;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub(crate) struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub(crate) trait Measure {
    /// False for elements hidden by `if`.
    fn visible(&self, n: &Node) -> bool;
    /// Natural size of a `<text>` laid out within `max_w`.
    fn text(&self, n: &Node, max_w: f32) -> (f32, f32);
}

#[derive(Clone, Copy, PartialEq)]
enum Dim {
    Auto,
    Fixed(f32),
    Fill,
}

fn dim(n: &Node, k: &str) -> Dim {
    match n.attr(k) {
        Some("fill") => Dim::Fill,
        Some(v) => v.trim().parse().map(Dim::Fixed).unwrap_or(Dim::Auto),
        None => Dim::Auto,
    }
}

fn num(n: &Node, k: &str, default: f32) -> f32 {
    n.attr(k).and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

/// (top, right, bottom, left)
fn padding(n: &Node) -> [f32; 4] {
    let v: Vec<f32> = n.attr("padding").unwrap_or("").split_whitespace().filter_map(|s| s.parse().ok()).collect();
    match v[..] {
        [a] => [a; 4],
        [a, b] => [a, b, a, b],
        [t, r, b, l] => [t, r, b, l],
        _ => [0.0; 4],
    }
}

fn is_shape(n: &Node) -> bool {
    matches!(n.tag.as_str(), "rect" | "ellipse" | "image" | "canvas")
}

/// All visible elements with their boxes, parents before children (draw order).
pub(crate) fn layout<'a>(root: &'a Node, bounds: Rect, m: &dyn Measure) -> Vec<(&'a Node, Rect)> {
    let mut out = Vec::new();
    if m.visible(root) {
        arrange(root, bounds, m, &mut out);
    }
    out
}

fn kids<'a>(n: &'a Node, m: &dyn Measure) -> Vec<&'a Node> {
    n.children.iter().filter(|c| m.visible(c)).collect()
}

/// Natural size, padding and fixed sizes included.
fn measure(n: &Node, max_w: f32, m: &dyn Measure) -> (f32, f32) {
    let [t, r, b, l] = padding(n);
    let inner = (max_w - l - r).max(0.0);
    let gap = num(n, "gap", 0.0);
    let ks = kids(n, m);
    let gaps = gap * ks.len().saturating_sub(1) as f32;
    let (w, h) = match n.tag.as_str() {
        "text" => m.text(n, inner),
        "row" => ks.iter().map(|k| measure(k, f32::INFINITY, m)).fold((gaps, 0.0f32), |(w, h), (kw, kh)| (w + kw, h.max(kh))),
        "column" => ks.iter().map(|k| measure(k, inner, m)).fold((0.0f32, gaps), |(w, h), (kw, kh)| (w.max(kw), h + kh)),
        "stack" => ks.iter().map(|k| measure(k, inner, m)).fold((0.0f32, 0.0f32), |(w, h), (kw, kh)| (w.max(kw), h.max(kh))),
        _ => (0.0, 0.0),
    };
    let (mut w, mut h) = (w + l + r, h + t + b);
    if let Dim::Fixed(v) = dim(n, "width") {
        w = v;
    }
    if let Dim::Fixed(v) = dim(n, "height") {
        h = v;
    }
    (w, h)
}

fn arrange<'a>(n: &'a Node, rect: Rect, m: &dyn Measure, out: &mut Vec<(&'a Node, Rect)>) {
    out.push((n, rect));
    let [t, r, b, l] = padding(n);
    let c = Rect { x: rect.x + l, y: rect.y + t, w: (rect.w - l - r).max(0.0), h: (rect.h - t - b).max(0.0) };
    let ks = kids(n, m);
    match n.tag.as_str() {
        "row" | "column" => line(n, c, &ks, n.tag == "row", m, out),
        "stack" => {
            for k in ks {
                let (nw, nh) = measure(k, c.w, m);
                let size = |d: Dim, natural: f32, avail: f32| match d {
                    Dim::Fill => avail,
                    Dim::Auto if is_shape(k) => avail,
                    _ => natural.min(avail),
                };
                let w = size(dim(k, "width"), nw, c.w);
                let h = size(dim(k, "height"), nh, c.h);
                let x = c.x + place(n.attr("align").unwrap_or("center"), c.w - w);
                let y = c.y + place(n.attr("justify").unwrap_or("center"), c.h - h);
                arrange(k, Rect { x, y, w, h }, m, out);
            }
        }
        _ => {}
    }
}

/// Offset of a box with `free` space left over.
fn place(how: &str, free: f32) -> f32 {
    match how {
        "center" => free / 2.0,
        "end" => free,
        _ => 0.0,
    }
}

fn line<'a>(n: &Node, c: Rect, ks: &[&'a Node], row: bool, m: &dyn Measure, out: &mut Vec<(&'a Node, Rect)>) {
    let (main_len, cross_len) = if row { (c.w, c.h) } else { (c.h, c.w) };
    let (main_key, cross_key) = if row { ("width", "height") } else { ("height", "width") };
    let gap = num(n, "gap", 0.0);
    let natural: Vec<(f32, f32)> = ks
        .iter()
        .map(|k| {
            let (w, h) = measure(k, if row { f32::INFINITY } else { c.w }, m);
            if row { (w, h) } else { (h, w) } // (main, cross)
        })
        .collect();
    let grow: Vec<f32> = ks
        .iter()
        .map(|k| match (k.attr("grow"), dim(k, main_key)) {
            (Some(g), _) => g.parse().unwrap_or(0.0),
            (None, Dim::Fill) => 1.0,
            _ => 0.0,
        })
        .collect();
    let total_grow: f32 = grow.iter().sum();
    let used: f32 = natural.iter().zip(&grow).filter(|(_, g)| **g == 0.0).map(|(s, _)| s.0).sum::<f32>()
        + gap * ks.len().saturating_sub(1) as f32;
    let free = (main_len - used).max(0.0);

    let (mut pos, mut between) = (0.0, 0.0);
    if total_grow == 0.0 {
        match n.attr("justify").unwrap_or("start") {
            "space-between" if ks.len() > 1 => between = free / (ks.len() - 1) as f32,
            how => pos = place(how, free),
        }
    }
    let align = n.attr("align").unwrap_or("start");
    for (i, k) in ks.iter().enumerate() {
        let main = if grow[i] > 0.0 { free * grow[i] / total_grow } else { natural[i].0 };
        let stretch = align == "stretch"
            || dim(k, cross_key) == Dim::Fill
            || (dim(k, cross_key) == Dim::Auto && is_shape(k));
        let cross = if stretch { cross_len } else { natural[i].1.min(cross_len) };
        let off = if stretch { 0.0 } else { place(align, cross_len - cross) };
        let r = if row {
            Rect { x: c.x + pos, y: c.y + off, w: main, h: cross }
        } else {
            Rect { x: c.x + off, y: c.y + pos, w: cross, h: main }
        };
        arrange(k, r, m, out);
        pos += main + gap + between;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Text is 10 DIPs per character, 20 DIPs tall. `if="no"` hides an element.
    struct Fake;
    impl Measure for Fake {
        fn visible(&self, n: &Node) -> bool {
            n.attr("if") != Some("no")
        }
        fn text(&self, n: &Node, _max_w: f32) -> (f32, f32) {
            (n.attr("value").unwrap_or("").len() as f32 * 10.0, 20.0)
        }
    }

    fn el(tag: &str, attrs: &[(&str, &str)], children: Vec<Node>) -> Node {
        let attrs: BTreeMap<String, String> = attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Node { tag: tag.into(), attrs, children, line: 0, id: 0 }
    }

    fn boxes(root: &Node, w: f32, h: f32) -> Vec<Rect> {
        layout(root, Rect { x: 0.0, y: 0.0, w, h }, &Fake).into_iter().map(|(_, r)| r).collect()
    }

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn row_with_grow_and_padding() {
        let root = el(
            "row",
            &[("padding", "10"), ("gap", "5")],
            vec![el("text", &[("value", "abcd")], vec![]), el("spacer", &[("grow", "1")], vec![]), el("rect", &[("width", "30")], vec![])],
        );
        let b = boxes(&root, 200.0, 100.0);
        assert_eq!(b[1], r(10.0, 10.0, 40.0, 20.0)); // text: natural, top-aligned
        assert_eq!(b[2], r(55.0, 10.0, 100.0, 0.0)); // spacer takes the rest: 180 - 40 - 30 - 2 gaps
        assert_eq!(b[3], r(160.0, 10.0, 30.0, 80.0)); // rect stretches on the cross axis
    }

    #[test]
    fn column_justify_and_hidden() {
        let root = el(
            "column",
            &[("justify", "center"), ("align", "center")],
            vec![el("text", &[("value", "ab")], vec![]), el("text", &[("value", "zzz"), ("if", "no")], vec![])],
        );
        let b = boxes(&root, 100.0, 100.0);
        assert_eq!(b.len(), 2, "hidden child is skipped");
        assert_eq!(b[1], r(40.0, 40.0, 20.0, 20.0));
    }

    #[test]
    fn stack_centers_and_fills() {
        let root = el(
            "stack",
            &[],
            vec![el("image", &[("src", "a.svg")], vec![]), el("rect", &[("width", "20"), ("height", "10")], vec![])],
        );
        let b = boxes(&root, 100.0, 50.0);
        assert_eq!(b[1], r(0.0, 0.0, 100.0, 50.0));
        assert_eq!(b[2], r(40.0, 20.0, 20.0, 10.0));
    }

    #[test]
    fn space_between() {
        let root = el("row", &[("justify", "space-between")], vec![el("text", &[("value", "a")], vec![]), el("text", &[("value", "b")], vec![])]);
        let b = boxes(&root, 100.0, 20.0);
        assert_eq!(b[2].x, 90.0);
    }
}
