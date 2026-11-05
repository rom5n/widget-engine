//! A widget defined by a package: layout.xml drawn with engine elements, no Rust code.
//!
//! Bindings: `{clock.time}`, `{opt.date}`, `{theme.dark}` inside any attribute.
//! Conditions: `if="opt.date"` shows the element when the value is not off/false/0/empty,
//! `if="!opt.date"` the opposite.
//! Rotation: `rotate="{clock.second_angle}"` turns the element around its center.
//! `spin="6"` keeps it turning at 6°/s on the compositor, re-synced every tick.
//! SVG images may use the same `{...}` bindings, plus `{accent}`, `{foreground}`, `{secondary}`
//! (as #RRGGBB), so artwork follows the theme.
//! `fill`, `stroke`, and text `color` take colors or gradients, `shadow` takes drop shadows,
//! see [`crate::paint`]. `opacity` is applied by the compositor.
//! `<style class="name" .../>` elements in `<widget>` share attributes: `class="name other"`.
//! States: `hover.fill="..."`, `pressed.scale="0.96"` apply while the pointer is over the element
//! or presses it. `opacity` and `scale` move with `transition="200"` (ms, ease-out) or `"spring"`
//! on the compositor. Opacity multiplies down the tree; scale applies to the element and everything
//! in it. `action="name"` fires on click, see [`crate::Scene::action`].
//! `drag="seek"` makes a slider: while the button is held, `{drag.seek}` is the pointer's place
//! across the element in percent (0-100), and the element keeps its `hover.` and `pressed.`
//! states; on release the action `seek:<fraction 0-1>` fires.
//! `<canvas id="graph"/>` draws with a callback from [`crate::Engine::on_paint`].

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::core::*;
use windows_numerics::{Matrix3x2, Vector2};

use crate::layout::{self, Measure, Rect};
use crate::package::{Node, Package, STATES, invalid};
use crate::paint::{self, Paint, parse_paint};
use crate::scene::Transition;
use crate::settings::Font;
use crate::{Customization, Event, LayerId, Scene, Theme, Widget, rgba, time};

/// Paint callback of a `<canvas>`: context, width, height (DIPs, origin at the element's top-left).
pub type PaintFn = Box<dyn FnMut(&ID2D1DeviceContext, f32, f32) -> Result<()>>;
/// Canvas callbacks by canvas id. Shared with the engine, which registers them.
pub(crate) type Canvases = Rc<RefCell<BTreeMap<String, PaintFn>>>;
/// Values the app sets with `Engine::set` / [`crate::set`], bound as `{app.<key>}`.
pub(crate) type AppValues = Rc<RefCell<BTreeMap<String, String>>>;

pub(crate) struct Markup {
    pkg: Package,
    canvases: Canvases,
    app: AppValues,
    /// Parent of each node, by node id.
    parents: Vec<Option<usize>>,
    drawn: Vec<Drawn>,
    /// Layout of the last build: (node id, box). A change means a full rebuild.
    boxes: Vec<(usize, Rect)>,
    /// Pointer position in widget DIPs while it is over the widget.
    pointer: Option<(f32, f32)>,
    /// Element under the pointer when the button went down.
    pressed: Option<usize>,
    /// Place of the pointer across the pressed `drag` element, 0-1, while it is held.
    drag: Option<f32>,
    /// Some attribute binds a `cal.` value: compute the month grid on each tick.
    uses_calendar: bool,
}

struct Drawn {
    id: usize,
    layer: LayerId,
    /// Interpolated attributes. A change means a redraw of this layer.
    sig: String,
    /// Room for shadows on each side of the element box, in DIPs.
    margin: f32,
}

type Values = BTreeMap<String, String>;

impl Markup {
    pub fn new(pkg: Package, canvases: Canvases, app: AppValues) -> Self {
        fn walk(n: &Node, parent: Option<usize>, out: &mut Vec<Option<usize>>) {
            if out.len() <= n.id {
                out.resize(n.id + 1, None);
            }
            out[n.id] = parent;
            n.children.iter().for_each(|c| walk(c, Some(n.id), out));
        }
        let mut parents = Vec::new();
        walk(&pkg.root, None, &mut parents);
        fn binds_calendar(n: &Node) -> bool {
            n.attrs.values().any(|v| v.contains("cal.")) || n.children.iter().any(binds_calendar)
        }
        let uses_calendar = binds_calendar(&pkg.root);
        Self { pkg, canvases, app, parents, drawn: Vec::new(), boxes: Vec::new(), pointer: None, pressed: None, drag: None, uses_calendar }
    }

    fn values(&self, scene: &Scene) -> Values {
        let now = time::now();
        let mut v: Values = time::clock_values(&now).into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        if self.uses_calendar {
            v.extend(time::calendar_values(&now));
        }
        for o in &self.pkg.customization.options {
            v.insert(format!("opt.{}", o.key), scene.option(&o.key).to_string());
        }
        v.insert("theme.dark".into(), if scene.theme().dark { "on" } else { "off" }.into());
        for (k, t) in &self.pkg.texts {
            v.insert(format!("str.{k}"), t.clone());
        }
        for (k, val) in self.app.borrow().iter() {
            v.insert(format!("app.{k}"), val.clone());
        }
        if let (Some(f), Some(name)) = (self.drag, self.pressed.and_then(|id| node_by_id(&self.pkg.root, id)).and_then(|n| n.attr("drag"))) {
            v.insert(format!("drag.{name}"), format!("{:.2}", f * 100.0));
        }
        v
    }

    /// Place of `x` across the `drag` element `id`, 0-1. None: not a drag element.
    fn drag_at(&self, id: usize, x: f32) -> Option<f32> {
        node_by_id(&self.pkg.root, id)?.attr("drag")?;
        let r = self.rect_of(id)?;
        Some(((x - r.x) / r.w.max(1.0)).clamp(0.0, 1.0))
    }

    fn laid_out<'a>(pkg: &'a Package, scene: &Scene, values: &Values) -> Vec<(&'a Node, Rect)> {
        let (w, h) = scene.size();
        let m = Measurer { scene, values };
        layout::layout(&pkg.root, Rect { x: 0.0, y: 0.0, w, h }, &m)
    }

    fn rect_of(&self, id: usize) -> Option<Rect> {
        self.boxes.iter().find(|(i, _)| *i == id).map(|(_, r)| *r)
    }

    fn under_pointer(&self, id: usize) -> bool {
        let (Some((x, y)), Some(r)) = (self.pointer, self.rect_of(id)) else { return false };
        x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
    }

    /// The node with its `hover.` / `pressed.` attributes applied while those states are on.
    fn styled<'a>(&self, n: &'a Node) -> Cow<'a, Node> {
        // A held slider keeps its states while the pointer wanders off it.
        let dragging = self.drag.is_some() && self.pressed == Some(n.id);
        let hover = dragging || self.under_pointer(n.id);
        let pressed = dragging || (hover && self.pressed == Some(n.id));
        if !hover || !n.attrs.keys().any(|k| k.contains('.')) {
            return Cow::Borrowed(n);
        }
        let mut s = n.clone();
        for (state, on) in STATES.iter().zip([hover, pressed]) {
            if !on {
                continue;
            }
            let prefix = format!("{state}.");
            for (k, v) in &n.attrs {
                if let Some(base) = k.strip_prefix(&prefix) {
                    s.attrs.insert(base.to_string(), v.clone());
                }
            }
        }
        Cow::Owned(s)
    }

    /// Elements with state styles that are hovered, and the pressed one.
    fn states(&self) -> (Vec<usize>, Option<usize>) {
        let hovered = self
            .boxes
            .iter()
            .filter(|(id, _)| self.under_pointer(*id))
            .filter(|(id, _)| node_by_id(&self.pkg.root, *id).is_some_and(|n| n.attrs.keys().any(|k| k.contains('.'))))
            .map(|(id, _)| *id)
            .collect();
        (hovered, self.pressed)
    }

    /// Topmost visible element at (x, y) that reacts to clicks.
    fn clickable_at(&self, x: f32, y: f32) -> Option<usize> {
        let clickable = |n: &Node| n.attr("action").is_some() || n.attr("drag").is_some() || n.attrs.keys().any(|k| k.starts_with("pressed."));
        self.boxes
            .iter()
            .rev()
            .filter(|(_, r)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
            .map(|(id, _)| *id)
            .find(|id| node_by_id(&self.pkg.root, *id).is_some_and(clickable))
    }

    /// Redraws changed elements, re-syncs compositor properties, and schedules the next tick.
    fn tick(&mut self, scene: &mut Scene, force: bool) -> Result<()> {
        let values = self.values(scene);
        let laid = Self::laid_out(&self.pkg, scene, &values);
        let boxes: Vec<(usize, Rect)> = laid.iter().map(|(n, r)| (n.id, *r)).collect();
        if !force && boxes != self.boxes {
            // Text grew or an element appeared: lay everything out again.
            scene.request_rebuild();
            return Ok(());
        }
        let theme = *scene.theme();
        for i in 0..self.drawn.len() {
            let (id, layer, margin) = (self.drawn[i].id, self.drawn[i].layer, self.drawn[i].margin);
            let Some((node, rect)) = laid.iter().find(|(n, _)| n.id == id) else { continue };
            let node = self.styled(node);
            let sig = signature(&node, &values, &self.pkg);
            if force || sig != self.drawn[i].sig {
                let shadows = shadows(&node, &values, &theme);
                draw(scene, layer, &node, *rect, &values, &self.pkg, &self.canvases, &shadows, margin)?;
                self.drawn[i].sig = sig;
            }
            // Opacity multiplies down the tree. Scale comes from the nearest element that sets it.
            let mut opacity = 1.0;
            let mut how = Transition::None;
            let mut scale: Option<(f32, Rect, Transition)> = None;
            let mut at = Some(id);
            while let Some(a) = at {
                let Some(raw) = node_by_id(&self.pkg.root, a) else { break };
                // Set in any state: the property must also go back when the state ends.
                let sets = |p: &str| raw.attrs.keys().any(|k| k == p || k.strip_suffix(p).is_some_and(|s| s.ends_with('.')));
                let n = self.styled(raw);
                let t = n.attr("transition").map(Transition::parse).unwrap_or_default();
                if sets("opacity") {
                    opacity *= num_attr(&n, "opacity", &values, 1.0);
                    if how == Transition::None {
                        how = t;
                    }
                }
                if scale.is_none() && sets("scale") {
                    scale = Some((num_attr(&n, "scale", &values, 1.0), self.rect_of(a).unwrap_or(*rect), t));
                }
                at = self.parents.get(a).copied().flatten();
            }
            scene.set_opacity(layer, opacity, how)?;
            if let Some((k, r, t)) = scale {
                // Center of the scaling element, in this layer's coordinates.
                let (cx, cy) = (r.x + r.w / 2.0 - rect.x + margin, r.y + r.h / 2.0 - rect.y + margin);
                scene.set_scale(layer, cx, cy, k, t)?;
            }
            if let Some(angle) = node.attr("rotate").map(|r| interp(r, &values)) {
                let angle: f32 = angle.trim().parse().unwrap_or(0.0);
                let (cx, cy) = (margin + rect.w / 2.0, margin + rect.h / 2.0);
                match node.attr("spin").and_then(|s| s.parse::<f32>().ok()).filter(|s| *s > 0.0) {
                    Some(rate) => scene.spin(layer, cx, cy, angle, rate)?,
                    None => scene.rotate(layer, cx, cy, angle)?,
                }
            }
        }
        let period = if uses_seconds(&self.pkg.root) { 1_000 } else { 60_000 };
        // +20 ms: a timer that fires a tick early would redraw the old value.
        scene.set_timer(time::ms_until_next(&time::now(), period) + 20);
        Ok(())
    }
}

impl Widget for Markup {
    fn size(&self) -> (f32, f32) {
        self.pkg.size
    }

    fn customization(&self) -> Customization {
        self.pkg.customization.clone()
    }

    fn interactive(&self, x: f32, y: f32) -> bool {
        self.clickable_at(x, y).is_some()
    }

    fn build(&mut self, scene: &mut Scene) -> Result<()> {
        let values = self.values(scene);
        let theme = *scene.theme();
        let laid = Self::laid_out(&self.pkg, scene, &values);
        self.boxes = laid.iter().map(|(n, r)| (n.id, *r)).collect();
        let mut drawn = Vec::new();
        // ponytail: one layer per drawn element. Merge static elements into one surface if
        // widgets with many elements show up in memory profiles.
        for (node, rect) in &laid {
            if draws(node) {
                let m = margin(node, &values, &theme);
                let layer = scene.add_layer(rect.x - m, rect.y - m, rect.w + 2.0 * m, rect.h + 2.0 * m)?;
                drawn.push(Drawn { id: node.id, layer, sig: String::new(), margin: m });
            }
        }
        drop(laid);
        self.drawn = drawn;
        self.tick(scene, true)
    }

    fn on_event(&mut self, event: Event, scene: &mut Scene) -> Result<()> {
        let before = (self.states(), self.drag);
        match event {
            Event::Timer | Event::TimeChanged | Event::DataChanged => return self.tick(scene, false),
            Event::Wheel { .. } | Event::Char(_) => return Ok(()),
            Event::PointerMove { x, y } => {
                self.pointer = Some((x, y));
                if self.drag.is_some() {
                    self.drag = self.pressed.and_then(|id| self.drag_at(id, x));
                }
            }
            Event::PointerLeave => (self.pointer, self.pressed, self.drag) = (None, None, None),
            Event::PointerDown { x, y } => {
                self.pointer = Some((x, y));
                self.pressed = self.clickable_at(x, y);
                self.drag = self.pressed.and_then(|id| self.drag_at(id, x));
            }
            Event::PointerUp { x, y } => {
                self.pointer = Some((x, y));
                if let Some(f) = self.drag.take() {
                    let name = self.pressed.take().and_then(|id| node_by_id(&self.pkg.root, id)).and_then(|n| n.attr("drag"));
                    if let Some(name) = name {
                        scene.action(&format!("{name}:{f:.4}"));
                    }
                } else if let Some(id) = self.pressed.take().filter(|id| self.under_pointer(*id)) {
                    if let Some(a) = node_by_id(&self.pkg.root, id).and_then(|n| n.attr("action")) {
                        let a = interp(a, &self.values(scene));
                        scene.action(&a);
                    }
                }
            }
        }
        // Most pointer moves change nothing: skip the layout pass.
        if (self.states(), self.drag) == before {
            return Ok(());
        }
        self.tick(scene, false)
    }
}

fn node_by_id(n: &Node, id: usize) -> Option<&Node> {
    if n.id == id {
        return Some(n);
    }
    // Ids are pre-order: the target is under the last child whose id is not above it.
    n.children.iter().take_while(|c| c.id <= id).last().and_then(|c| node_by_id(c, id))
}

/// Room for the largest shadow the element has in any state.
fn margin(n: &Node, values: &Values, theme: &Theme) -> f32 {
    ["shadow", "hover.shadow", "pressed.shadow"]
        .iter()
        .filter_map(|k| n.attr(k))
        .map(|s| paint::shadow_margin(&paint::parse_shadows(&interp(s, values), theme)))
        .fold(0.0, f32::max)
}

struct Measurer<'a> {
    scene: &'a Scene,
    values: &'a Values,
}

impl Measure for Measurer<'_> {
    fn visible(&self, n: &Node) -> bool {
        n.attr("if").is_none_or(|c| condition(c, self.values))
    }

    fn text(&self, n: &Node, max_w: f32) -> (f32, f32) {
        let Ok(layout) = text_layout(self.scene, n, self.values, max_w, f32::MAX) else { return (0.0, 0.0) };
        let mut m = DWRITE_TEXT_METRICS::default();
        if unsafe { layout.GetMetrics(&mut m) }.is_err() {
            return (0.0, 0.0);
        }
        // One line cut with "…" to its box: it asks for no width, the box comes from width/grow.
        let w = if n.attr("ellipsis") == Some("true") { 0.0 } else { m.widthIncludingTrailingWhitespace.ceil() };
        (w, m.height.ceil())
    }
}

/// Elements that get their own layer. Containers only when they paint something.
fn draws(n: &Node) -> bool {
    match n.tag.as_str() {
        "text" | "image" | "rect" | "ellipse" | "canvas" => true,
        "row" | "column" | "stack" => n.attrs.keys().any(|k| {
            let base = k.rsplit('.').next().unwrap_or(k);
            matches!(base, "fill" | "stroke" | "shadow")
        }),
        _ => false,
    }
}

/// Replaces every `{key}` with its value. Unknown keys become empty.
pub(crate) fn interpolate(s: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        match rest[start..].find('}') {
            Some(end) => {
                out.push_str(&lookup(rest[start + 1..start + end].trim()).unwrap_or_default());
                rest = &rest[start + end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn interp(s: &str, values: &Values) -> String {
    interpolate(s, &|k| values.get(k).cloned())
}

fn condition(c: &str, values: &Values) -> bool {
    let (neg, key) = match c.trim().strip_prefix('!') {
        Some(k) => (true, k.trim()),
        None => (false, c.trim()),
    };
    let v = values.get(key).map(String::as_str).unwrap_or("");
    let truthy = !matches!(v, "" | "off" | "false" | "0");
    truthy != neg
}

fn signature(n: &Node, values: &Values, pkg: &Package) -> String {
    let mut sig: String = n.attrs.iter().filter(|(k, _)| !k.contains('.') && !matches!(k.as_str(), "rotate" | "spin" | "opacity" | "scale" | "transition" | "action")).map(|(k, v)| format!("{k}={}\u{1}", interp(v, values))).collect();
    // An SVG with bindings changes when its values do, not its src.
    if n.tag == "image" {
        let src = interp(n.attr("src").unwrap_or(""), values);
        if src.to_ascii_lowercase().ends_with(".svg") {
            let raw = svg_text(&pkg.dir.join(src));
            sig += &interpolate(&raw, &|k| values.get(k).cloned());
        }
    }
    sig
}

/// An SVG file's text, read again only when the file changes (hot reload).
fn svg_text(path: &std::path::Path) -> String {
    thread_local! {
        static CACHE: std::cell::RefCell<std::collections::HashMap<std::path::PathBuf, (Option<std::time::SystemTime>, String)>> = Default::default();
    }
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    CACHE.with_borrow_mut(|c| {
        if let Some((_, text)) = c.get(path).filter(|(t, _)| *t == modified) {
            return text.clone();
        }
        let text = std::fs::read_to_string(path).unwrap_or_default();
        c.insert(path.to_path_buf(), (modified, text.clone()));
        text
    })
}

fn uses_seconds(n: &Node) -> bool {
    let text_uses = n.tag == "text"
        && n.attr("value").is_some_and(|v| v.contains("clock.second}") || v.contains("clock.time_seconds"));
    text_uses || n.children.iter().any(uses_seconds)
}

/// Theme tokens (foreground, secondary, accent, transparent) or #RGB, #RRGGBB, #RRGGBBAA.
pub(crate) fn parse_color(s: &str, theme: &Theme) -> Option<D2D1_COLOR_F> {
    let fg = theme.foreground();
    match s.trim() {
        "foreground" => return Some(fg),
        "secondary" => return Some(D2D1_COLOR_F { a: fg.a * if theme.dark { 0.65 } else { 0.6 }, ..fg }),
        "accent" => return Some(theme.accent),
        "transparent" => return Some(rgba(0.0, 0.0, 0.0, 0.0)),
        _ => {}
    }
    let hex = s.trim().strip_prefix('#')?;
    let digits: Vec<u8> = hex.chars().map(|c| c.to_digit(16).map(|d| d as u8)).collect::<Option<_>>()?;
    let byte = |hi: u8, lo: u8| (hi * 16 + lo) as f32 / 255.0;
    match digits[..] {
        [r, g, b] => Some(rgba(byte(r, r), byte(g, g), byte(b, b), 1.0)),
        [r1, r2, g1, g2, b1, b2] => Some(rgba(byte(r1, r2), byte(g1, g2), byte(b1, b2), 1.0)),
        [r1, r2, g1, g2, b1, b2, a1, a2] => Some(rgba(byte(r1, r2), byte(g1, g2), byte(b1, b2), byte(a1, a2))),
        _ => None,
    }
}

fn hex(c: D2D1_COLOR_F) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02X}{:02X}{:02X}", b(c.r), b(c.g), b(c.b))
}

fn paint_attr(n: &Node, k: &str, values: &Values, theme: &Theme) -> Option<Paint> {
    n.attr(k).and_then(|v| parse_paint(&interp(v, values), theme))
}

fn shadows(n: &Node, values: &Values, theme: &Theme) -> Vec<paint::Shadow> {
    n.attr("shadow").map(|s| paint::parse_shadows(&interp(s, values), theme)).unwrap_or_default()
}

fn num_attr(n: &Node, k: &str, values: &Values, default: f32) -> f32 {
    n.attr(k).and_then(|v| interp(v, values).trim().parse().ok()).unwrap_or(default)
}

/// Text layout in the user's font unless the element sets its own family or weight.
fn text_layout(scene: &Scene, n: &Node, values: &Values, max_w: f32, max_h: f32) -> Result<IDWriteTextLayout> {
    let font: &Font = scene.font();
    let family = n.attr("family").map(|f| interp(f, values)).unwrap_or_else(|| font.family.clone());
    let weight = n.attr("weight").and_then(|w| interp(w, values).trim().parse().ok()).map(DWRITE_FONT_WEIGHT).unwrap_or(font.weight);
    let size = num_attr(n, "size", values, 14.0);
    let text: Vec<u16> = interp(n.attr("value").unwrap_or(""), values).encode_utf16().collect();
    unsafe {
        let dw = scene.dwrite();
        let format = crate::fonts::text_format(dw, &family, weight, font.stretch, size)?;
        format.SetTextAlignment(match n.attr("text-align") {
            Some("center") => DWRITE_TEXT_ALIGNMENT_CENTER,
            Some("end") => DWRITE_TEXT_ALIGNMENT_TRAILING,
            _ => DWRITE_TEXT_ALIGNMENT_LEADING,
        })?;
        let fit = n.attr("fit") == Some("true");
        if fit {
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        }
        if n.attr("ellipsis") == Some("true") {
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            let sign = dw.CreateEllipsisTrimmingSign(&format)?;
            let trimming = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, ..Default::default() };
            format.SetTrimming(&trimming, &sign)?;
        }
        let layout = dw.CreateTextLayout(&text, &format, max_w.max(0.0), max_h.max(0.0))?;
        let all = DWRITE_TEXT_RANGE { startPosition: 0, length: u32::MAX };
        if let Some(t) = n.attr("tracking").and_then(|t| t.trim().parse::<f32>().ok()) {
            layout.cast::<IDWriteTextLayout1>()?.SetCharacterSpacing(0.0, t, 0.0, all)?;
        }
        if let Some(features) = n.attr("features") {
            let typo = dw.CreateTypography()?;
            for tag in features.split([' ', ',']).filter(|t| t.len() == 4) {
                let b = tag.as_bytes();
                let code = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                typo.AddFontFeature(DWRITE_FONT_FEATURE { nameTag: DWRITE_FONT_FEATURE_TAG(code), parameter: 1 })?;
            }
            layout.SetTypography(&typo, all)?;
        }
        if fit && max_w.is_finite() && max_w > 0.0 {
            // One line: a font wider than the box gets a smaller size instead of a second line.
            let mut m = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut m)?;
            let natural = m.widthIncludingTrailingWhitespace;
            if natural > max_w {
                layout.SetFontSize(size * max_w / natural * 0.98, all)?;
            }
        }
        Ok(layout)
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(
    scene: &mut Scene,
    layer: LayerId,
    n: &Node,
    rect: Rect,
    values: &Values,
    pkg: &Package,
    canvases: &Canvases,
    shadows: &[paint::Shadow],
    margin: f32,
) -> Result<()> {
    let theme = *scene.theme();
    let d2d = scene.d2d().clone();
    let text = if n.tag == "text" { Some(text_layout(scene, n, values, rect.w, rect.h)?) } else { None };
    let (w, h) = (rect.w, rect.h);
    scene.draw(layer, |ctx| unsafe {
        let mut base = Matrix3x2::default();
        ctx.GetTransform(&mut base);
        ctx.SetTransform(&(Matrix3x2::translation(margin, margin) * base));
        paint::with_shadows(ctx, shadows, || draw_inner(ctx, &d2d, n, w, h, values, &theme, text.as_ref(), pkg, canvases))
    })
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_inner(
    ctx: &ID2D1DeviceContext,
    d2d: &ID2D1Factory1,
    n: &Node,
    w: f32,
    h: f32,
    values: &Values,
    theme: &Theme,
    text: Option<&IDWriteTextLayout>,
    pkg: &Package,
    canvases: &Canvases,
) -> Result<()> {
    unsafe {
        let fill = paint_attr(n, "fill", values, theme);
        let stroke = paint_attr(n, "stroke", values, theme);
        let sw = num_attr(n, "stroke-width", values, 1.0);
        let brush = |p: &Paint| paint::brush(ctx, p, w, h);
        match n.tag.as_str() {
            "text" => {
                let color = paint_attr(n, "color", values, theme).unwrap_or(Paint::Solid(parse_color("foreground", theme).unwrap()));
                if let Some(layout) = text {
                    ctx.DrawTextLayout(Vector2 { X: 0.0, Y: 0.0 }, layout, &brush(&color)?, D2D1_DRAW_TEXT_OPTIONS_NONE);
                }
            }
            "ellipse" => {
                let inset = if stroke.is_some() { sw / 2.0 } else { 0.0 };
                let e = D2D1_ELLIPSE { point: Vector2 { X: w / 2.0, Y: h / 2.0 }, radiusX: w / 2.0 - inset, radiusY: h / 2.0 - inset };
                if let Some(p) = &fill {
                    ctx.FillEllipse(&e, &brush(p)?);
                }
                if let Some(p) = &stroke {
                    ctx.DrawEllipse(&e, &brush(p)?, sw, None);
                }
            }
            "image" => draw_image(ctx, n, w, h, values, theme, pkg)?,
            "canvas" => {
                let id = n.attr("id").unwrap_or("");
                // Not registered (yet): an empty box.
                if let Some(paint) = canvases.borrow_mut().get_mut(id) {
                    paint(ctx, w, h)?;
                }
            }
            // rect and filled containers
            _ => {
                let inset = if stroke.is_some() { sw / 2.0 } else { 0.0 };
                let radius = num_attr(n, "radius", values, 0.0);
                let (x, y, gw, gh) = (inset, inset, w - 2.0 * inset, h - 2.0 * inset);
                let geo: ID2D1Geometry = if radius <= 0.0 {
                    d2d.CreateRectangleGeometry(&D2D_RECT_F { left: x, top: y, right: x + gw, bottom: y + gh })?.cast()?
                } else if n.attr("corners") == Some("round") {
                    d2d.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F { left: x, top: y, right: x + gw, bottom: y + gh },
                        radiusX: radius,
                        radiusY: radius,
                    })?
                    .cast()?
                } else {
                    crate::shapes::squircle(d2d, x, y, gw, gh, radius)?.cast()?
                };
                if let Some(p) = &fill {
                    ctx.FillGeometry(&geo, &brush(p)?, None);
                }
                if let Some(p) = &stroke {
                    ctx.DrawGeometry(&geo, &brush(p)?, sw, None);
                }
            }
        }
        Ok(())
    }
}

/// SVG (with bindings and theme colors) or a raster image, scaled to the element box.
unsafe fn draw_image(ctx: &ID2D1DeviceContext, n: &Node, w: f32, h: f32, values: &Values, theme: &Theme, pkg: &Package) -> Result<()> {
    let src = interp(n.attr("src").unwrap_or(""), values);
    let path = pkg.dir.join(&src);
    unsafe {
        // Rounded corners: plain ones (a circle at half the size) or the squircle.
        let radius = num_attr(n, "radius", values, 0.0);
        let clip: Option<ID2D1Geometry> = if radius > 0.0 {
            let d2d: ID2D1Factory1 = ctx.GetFactory()?.cast()?;
            Some(if n.attr("corners") == Some("round") {
                let r = radius.min(w / 2.0).min(h / 2.0);
                d2d.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT { rect: D2D_RECT_F { left: 0.0, top: 0.0, right: w, bottom: h }, radiusX: r, radiusY: r })?.cast()?
            } else {
                crate::shapes::squircle(&d2d, 0.0, 0.0, w, h, radius)?.cast()?
            })
        } else {
            None
        };
        if src.to_ascii_lowercase().ends_with(".svg") {
            let raw = std::fs::read_to_string(&path).map_err(|e| invalid(format!("layout.xml:{}: {}: {e}", n.line, path.display())))?;
            let svg = interpolate(&raw, &|k| {
                values.get(k).cloned().or_else(|| matches!(k, "accent" | "foreground" | "secondary").then(|| hex(parse_color(k, theme).unwrap())))
            });
            let stream = SHCreateMemStream(Some(svg.as_bytes())).ok_or_else(|| invalid("SHCreateMemStream failed"))?;
            let ctx5: ID2D1DeviceContext5 = ctx.cast()?;
            let doc = ctx5.CreateSvgDocument(&stream, D2D_SIZE_F { width: w, height: h })?;
            match &clip {
                Some(g) => {
                    let params = D2D1_LAYER_PARAMETERS1 {
                        contentBounds: D2D_RECT_F { left: 0.0, top: 0.0, right: w, bottom: h },
                        geometricMask: std::mem::ManuallyDrop::new(Some(g.clone())),
                        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                        maskTransform: Matrix3x2::identity(),
                        opacity: 1.0,
                        opacityBrush: std::mem::ManuallyDrop::new(None),
                        layerOptions: D2D1_LAYER_OPTIONS1_NONE,
                    };
                    ctx.PushLayer(&params, None);
                    ctx5.DrawSvgDocument(&doc);
                    ctx.PopLayer();
                    drop(std::mem::ManuallyDrop::into_inner(params.geometricMask));
                }
                None => ctx5.DrawSvgDocument(&doc),
            }
        } else {
            let image = crate::backdrop::load_image(ctx, &path.to_string_lossy(), 1.0)?;
            let size = image.GetPixelSize();
            let brush = ctx.CreateBitmapBrush(
                &image,
                Some(&D2D1_BITMAP_BRUSH_PROPERTIES1 {
                    extendModeX: D2D1_EXTEND_MODE_CLAMP,
                    extendModeY: D2D1_EXTEND_MODE_CLAMP,
                    interpolationMode: D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                }),
                None,
            )?;
            // Cover: fill the box, keep the aspect ratio, crop the overflow evenly.
            // Bitmap DPI is 96 by default: one bitmap pixel = one DIP before this scale.
            let (iw, ih) = (size.width as f32, size.height as f32);
            let k = (w / iw).max(h / ih);
            brush.SetTransform(&Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: (w - iw * k) / 2.0, M32: (h - ih * k) / 2.0 });
            match &clip {
                Some(g) => ctx.FillGeometry(g, &brush, None),
                None => ctx.FillRectangle(&D2D_RECT_F { left: 0.0, top: 0.0, right: w, bottom: h }, &brush),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vals(pairs: &[(&str, &str)]) -> Values {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn interpolation() {
        let v = vals(&[("clock.time", "13:05")]);
        assert_eq!(interp("Now {clock.time}!", &v), "Now 13:05!");
        assert_eq!(interp("{missing}x", &v), "x");
        assert_eq!(interp("open { brace", &v), "open { brace");
    }

    #[test]
    fn conditions() {
        let v = vals(&[("opt.date", "on"), ("opt.dial", "off")]);
        assert!(condition("opt.date", &v));
        assert!(!condition("opt.dial", &v));
        assert!(condition("!opt.dial", &v));
        assert!(!condition("opt.unknown", &v));
    }

    #[test]
    fn colors() {
        let t = Theme { dark: true, accent: rgba(0.0, 0.5, 1.0, 1.0), text: None };
        let c = parse_color("#FF000080", &t).unwrap();
        assert!((c.r - 1.0).abs() < 1e-6 && c.g == 0.0 && (c.a - 128.0 / 255.0).abs() < 1e-6);
        assert_eq!(parse_color("#0f0", &t).unwrap().g, 1.0);
        assert_eq!(parse_color("accent", &t).unwrap().b, 1.0);
        assert!(parse_color("#12", &t).is_none());
        assert_eq!(hex(rgba(1.0, 0.0, 0.5, 1.0)), "#FF0080");
    }
}
