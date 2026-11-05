//! Widget packages: a directory with `widget.toml` (manifest), `layout.xml` (markup), and assets.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::Graphics::DirectWrite::{DWRITE_FONT_STRETCH, DWRITE_FONT_WEIGHT};
use windows::core::{Error, Result};

use crate::settings::{Allow, Background, Card, Choice, Customization, Font};

pub(crate) struct Package {
    pub dir: PathBuf,
    pub id: String,
    /// Translated, like `category`.
    pub name: String,
    pub category: String,
    /// Translated, like `category`.
    pub variant: String,
    /// `{str.<key>}` values: every key of the `[strings.*]` tables, in the display language
    /// (the key itself when there is no translation).
    pub texts: BTreeMap<String, String>,
    pub size: (f32, f32),
    pub customization: Customization,
    pub root: Node,
}

/// One markup element.
#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub tag: String,
    pub attrs: BTreeMap<String, String>,
    pub children: Vec<Node>,
    /// 1-based line in layout.xml, for error messages.
    pub line: u32,
    /// Pre-order index, stable for the life of the package.
    pub id: usize,
}

impl Node {
    pub fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.get(k).map(String::as_str)
    }
}

pub(crate) fn invalid(msg: impl AsRef<str>) -> Error {
    Error::new(E_INVALIDARG, msg.as_ref())
}

pub(crate) fn load(dir: &Path) -> Result<Package> {
    let read = |name: &str| {
        std::fs::read_to_string(dir.join(name)).map_err(|e| invalid(format!("{}: {e}", dir.join(name).display())))
    };
    let manifest: Manifest =
        toml::from_str(&read("widget.toml")?).map_err(|e| invalid(format!("widget.toml: {e}")))?;
    check_engine_version(&manifest.engine)?;
    let root = parse_layout(&read("layout.xml")?)?;
    let strings = manifest.strings.get(crate::i18n::lang());
    let tr = |s: &str| strings.and_then(|m| m.get(s)).map_or(s, |v| v.as_str()).to_string();
    Ok(Package {
        dir: dir.to_path_buf(),
        id: manifest.id.clone(),
        name: tr(&manifest.name),
        category: manifest.category.as_deref().map(tr).unwrap_or_default(),
        variant: manifest.variant.as_deref().map(tr).unwrap_or_default(),
        texts: manifest.strings.values().flat_map(|m| m.keys()).map(|k| (k.clone(), tr(k))).collect(),
        size: (manifest.size[0], manifest.size[1]),
        customization: manifest.customization()?,
        root,
    })
}

/// Changes when any file in the package changes, appears, or goes away. For hot reload.
pub(crate) fn stamp(dir: &Path) -> (std::time::SystemTime, usize) {
    let mut newest = std::time::SystemTime::UNIX_EPOCH;
    let mut count = 0;
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                todo.push(e.path());
            } else {
                count += 1;
                newest = newest.max(meta.modified().unwrap_or(newest));
            }
        }
    }
    (newest, count)
}

// --- widget.toml -----------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    id: String,
    name: String,
    /// Gallery section, e.g. "Time". Free text.
    category: Option<String>,
    /// Which variant of a widget this is, e.g. "Small", shown under its name in a gallery.
    variant: Option<String>,
    /// Compatible engine versions, e.g. "^0.1".
    engine: String,
    /// Width and height in DIPs.
    size: [f32; 2],
    card: Option<CardToml>,
    font: Option<FontToml>,
    #[serde(default)]
    customize: CustomizeToml,
    #[serde(default)]
    option: Vec<OptionToml>,
    /// Translations of option labels: language ("ru", "de", ...) -> English label -> label.
    #[serde(default)]
    strings: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CardToml {
    radius: Option<f32>,
    background: Option<String>,
    border: Option<bool>,
    shadow: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FontToml {
    family: Option<String>,
    weight: Option<i32>,
    /// DWRITE_FONT_STRETCH: 1 ultra-condensed .. 5 normal .. 9 ultra-expanded.
    width: Option<i32>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct CustomizeToml {
    /// Any of "size", "background", "theme", "font", "shape", "colors", "text".
    #[serde(default)]
    allow: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionToml {
    key: String,
    label: String,
    /// [value, label] pairs. Missing: a Show/Hide toggle with values "on"/"off".
    values: Option<Vec<[String; 2]>>,
    default: Option<String>,
    /// "image" or "folder": the user picks a file or folder, bound as its path.
    kind: Option<String>,
    /// value -> [width, height]: picking it resizes the widget.
    sizes: Option<BTreeMap<String, [f32; 2]>>,
}

impl Manifest {
    fn customization(&self) -> Result<Customization> {
        let mut allow = Allow::default();
        for a in &self.customize.allow {
            match a.as_str() {
                "size" => allow.size = true,
                "background" => allow.background = true,
                "theme" => allow.theme = true,
                "font" => allow.font = true,
                "shape" => allow.shape = true,
                "colors" => allow.colors = true,
                "text" => allow.text = true,
                other => return Err(invalid(format!("widget.toml: unknown customize.allow entry '{other}'"))),
            }
        }
        let card = match &self.card {
            None => None,
            Some(c) => Some(Card {
                radius: c.radius.unwrap_or(Card::default().radius),
                border: c.border.unwrap_or(true),
                shadow: c.shadow.unwrap_or(false),
                background: match &c.background {
                    None => Card::default().background,
                    Some(b) => Background::parse(b)
                        .ok_or_else(|| invalid(format!("widget.toml: card.background '{b}': use frosted, glass, solid, or clear")))?,
                },
            }),
        };
        let mut font = Font::default();
        if let Some(f) = &self.font {
            if let Some(family) = &f.family {
                font.family = family.clone();
            }
            if let Some(w) = f.weight {
                font.weight = DWRITE_FONT_WEIGHT(w);
            }
            if let Some(w) = f.width {
                font.stretch = DWRITE_FONT_STRETCH(w);
            }
        }
        let strings = self.strings.get(crate::i18n::lang());
        // Option labels the package does not translate may be engine texts (city names).
        let tr = |s: &String| strings.and_then(|m| m.get(s)).map_or_else(|| crate::i18n::t(s).to_string(), |v| v.clone());
        let options = self
            .option
            .iter()
            .map(|o| {
                let kind = match o.kind.as_deref() {
                    None => crate::ChoiceKind::List,
                    Some("image") => crate::ChoiceKind::Image,
                    Some("folder") => crate::ChoiceKind::Folder,
                    Some(k) => return Err(invalid(format!("widget.toml: option '{}': kind '{k}' is not image or folder", o.key))),
                };
                if kind != crate::ChoiceKind::List {
                    return Ok(Choice { key: o.key.clone(), label: tr(&o.label), values: Vec::new(), default: String::new(), kind, sizes: Vec::new() });
                }
                let values: Vec<(String, String)> = match &o.values {
                    Some(v) => v.iter().map(|[a, b]| (a.clone(), tr(b))).collect(),
                    None => vec![("on".into(), "Show".into()), ("off".into(), "Hide".into())],
                };
                let default = o.default.clone().unwrap_or_else(|| values[0].0.clone());
                if !values.iter().any(|(v, _)| *v == default) {
                    return Err(invalid(format!("widget.toml: option '{}': default '{default}' is not one of its values", o.key)));
                }
                let sizes = o.sizes.iter().flatten().map(|(v, [w, h])| (v.clone(), (*w, *h))).collect();
                Ok(Choice { key: o.key.clone(), label: tr(&o.label), values, default, kind, sizes })
            })
            .collect::<Result<_>>()?;
        Ok(Customization { card, allow, font, options })
    }
}

/// "^0.1" matches engine 0.1.x. Before 1.0, the minor version is the breaking one.
fn check_engine_version(req: &str) -> Result<()> {
    let ours = env!("CARGO_PKG_VERSION");
    let want = req.trim().trim_start_matches('^');
    let prefix = |v: &str, n: usize| v.split('.').take(n).collect::<Vec<_>>().join(".");
    let n = if want.starts_with("0.") { 2 } else { 1 };
    if prefix(want, n) == prefix(ours, n) {
        Ok(())
    } else {
        Err(invalid(format!("widget.toml: engine = \"{req}\" does not match engine version {ours}")))
    }
}

// --- layout.xml ------------------------------------------------------------

/// Attributes every element accepts.
pub(crate) const COMMON: &[&str] = &[
    "width", "height", "grow", "padding", "gap", "align", "justify", "if", "class", "opacity", "scale", "rotate", "spin",
    "shadow", "transition", "action", "drag",
];
/// Attributes that place elements. They have no `hover.` / `pressed.` variants: states never move things.
const LAYOUT: &[&str] = &["width", "height", "grow", "padding", "gap", "align", "justify", "if", "class"];
/// Attributes that have no `hover.` / `pressed.` variants either.
const STATELESS: &[&str] = &["spin", "transition", "action", "drag", "id"];
const PANEL: &[&str] = &["fill", "stroke", "stroke-width", "radius", "corners"];
pub(crate) const STATES: &[&str] = &["hover", "pressed"];

/// Element names and their own attributes (besides [`COMMON`]).
pub(crate) const ELEMENTS: &[(&str, &[&str])] = &[
    ("row", PANEL),
    ("column", PANEL),
    ("stack", PANEL),
    ("text", &["value", "size", "weight", "family", "color", "text-align", "tracking", "features", "ellipsis", "fit"]),
    ("image", &["src", "radius", "corners"]),
    ("rect", PANEL),
    ("ellipse", &["fill", "stroke", "stroke-width"]),
    ("canvas", &["id", "value"]),
    ("spacer", &[]),
];

fn allowed(tag: &str) -> Option<&'static [&'static str]> {
    ELEMENTS.iter().find(|(t, _)| *t == tag).map(|(_, a)| *a)
}

/// Whether `<tag>` takes attribute `k`, including `hover.fill` style state variants.
pub(crate) fn takes(tag: &str, k: &str) -> bool {
    let Some(own) = allowed(tag) else { return false };
    let base = |k: &str| COMMON.contains(&k) || own.contains(&k);
    match k.split_once('.') {
        Some((state, attr)) => STATES.contains(&state) && base(attr) && has_states(attr),
        None => base(k),
    }
}

/// Attributes that may have `hover.` / `pressed.` variants.
pub(crate) fn has_states(attr: &str) -> bool {
    !LAYOUT.contains(&attr) && !STATELESS.contains(&attr)
}

/// XML Schema for layout.xml, generated from the same tables the parser checks against.
/// Editors with XML support (e.g. the Red Hat XML extension for VS Code) use it for completion
/// and errors as you type. `widget-dev --schema` prints it.
pub fn layout_schema() -> String {
    let mut x: Vec<String> = [
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        r#"<!-- Generated by widget-dev from the engine's element tables. Do not edit. -->"#,
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">"#,
        r#"  <xs:element name="widget">"#,
        r#"    <xs:complexType>"#,
        r#"      <xs:sequence>"#,
        r#"        <xs:element name="style" minOccurs="0" maxOccurs="unbounded">"#,
        r#"          <xs:complexType>"#,
        r#"            <xs:attribute name="class" type="xs:string" use="required"/>"#,
        r#"            <xs:anyAttribute processContents="skip"/>"#,
        r#"          </xs:complexType>"#,
        r#"        </xs:element>"#,
        r#"        <xs:group ref="element"/>"#,
        r#"      </xs:sequence>"#,
        r#"      <xs:anyAttribute namespace="http://www.w3.org/2001/XMLSchema-instance" processContents="skip"/>"#,
        r#"    </xs:complexType>"#,
        r#"  </xs:element>"#,
        r#"  <xs:group name="element">"#,
        r#"    <xs:choice>"#,
    ]
    .map(String::from)
    .to_vec();
    for (tag, _) in ELEMENTS {
        x.push(format!(r#"      <xs:element name="{tag}" type="{tag}"/>"#));
    }
    x.push("    </xs:choice>".into());
    x.push("  </xs:group>".into());
    for (tag, own) in ELEMENTS {
        x.push(format!(r#"  <xs:complexType name="{tag}">"#));
        if matches!(*tag, "row" | "column" | "stack") {
            x.push("    <xs:sequence>".into());
            x.push(r#"      <xs:group ref="element" minOccurs="0" maxOccurs="unbounded"/>"#.into());
            x.push("    </xs:sequence>".into());
        }
        let required = |a: &str| matches!((*tag, a), ("text", "value") | ("image", "src") | ("canvas", "id"));
        let attrs = || COMMON.iter().chain(own.iter());
        for a in attrs() {
            let usage = if required(a) { r#" use="required""# } else { "" };
            x.push(format!(r#"    <xs:attribute name="{a}" type="xs:string"{usage}/>"#));
        }
        for state in STATES {
            for a in attrs().filter(|a| has_states(a)) {
                x.push(format!(r#"    <xs:attribute name="{state}.{a}" type="xs:string"/>"#));
            }
        }
        x.push("  </xs:complexType>".into());
    }
    x.push("</xs:schema>".into());
    x.join("\n") + "\n"
}

fn parse_layout(text: &str) -> Result<Node> {
    let doc = roxmltree::Document::parse(text).map_err(|e| invalid(format!("layout.xml: {e}")))?;
    let root = doc.root_element();
    if root.tag_name().name() != "widget" {
        return Err(invalid("layout.xml: the root element must be <widget>"));
    }
    // <style class="name" .../> elements first: shared attributes for `class="name"`.
    let mut styles: Styles = BTreeMap::new();
    let mut body = Vec::new();
    for n in root.children().filter(|n| n.is_element()) {
        if n.tag_name().name() != "style" {
            body.push(n);
            continue;
        }
        let line = doc.text_pos_at(n.range().start).row;
        let name = n.attribute("class").ok_or_else(|| invalid(format!("layout.xml:{line}: <style> needs a class attribute")))?;
        let attrs = n.attributes().filter(|a| a.name() != "class").map(|a| (a.name().to_string(), a.value().to_string()));
        styles.insert(name.to_string(), attrs.collect());
    }
    let [first] = body[..] else {
        return Err(invalid("layout.xml: <widget> must contain exactly one element besides <style> elements"));
    };
    let mut next = 0;
    convert(&doc, first, &styles, &mut next)
}

type Styles = BTreeMap<String, BTreeMap<String, String>>;

fn convert(doc: &roxmltree::Document, n: roxmltree::Node, styles: &Styles, next: &mut usize) -> Result<Node> {
    let id = *next;
    *next += 1;
    let tag = n.tag_name().name().to_string();
    let line = doc.text_pos_at(n.range().start).row;
    let at = |msg: String| invalid(format!("layout.xml:{line}: {msg}"));
    allowed(&tag).ok_or_else(|| at(format!("unknown element <{tag}>")))?;
    // Classes in order, then the element's own attributes win.
    let mut attrs = BTreeMap::new();
    for class in n.attribute("class").unwrap_or("").split_whitespace() {
        let style = styles.get(class).ok_or_else(|| at(format!("no <style class=\"{class}\">")))?;
        attrs.extend(style.clone());
    }
    attrs.extend(n.attributes().filter(|a| a.name() != "class").map(|a| (a.name().to_string(), a.value().to_string())));
    for k in attrs.keys() {
        if !takes(&tag, k) {
            return Err(at(format!("<{tag}> has no attribute '{k}'")));
        }
    }
    let children =
        n.children().filter(|c| c.is_element()).map(|c| convert(doc, c, styles, next)).collect::<Result<Vec<_>>>()?;
    if !children.is_empty() && !matches!(tag.as_str(), "row" | "column" | "stack") {
        return Err(at(format!("<{tag}> cannot have child elements")));
    }
    if tag == "text" && !attrs.contains_key("value") {
        return Err(at("<text> needs a value attribute".into()));
    }
    if tag == "image" && !attrs.contains_key("src") {
        return Err(at("<image> needs a src attribute".into()));
    }
    if tag == "canvas" && !attrs.contains_key("id") {
        return Err(at("<canvas> needs an id attribute".into()));
    }
    Ok(Node { tag, attrs, children, line, id })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_errors_name_the_line() {
        let e = parse_layout("<widget>\n  <row>\n    <txt value='a'/>\n  </row>\n</widget>").unwrap_err();
        assert!(e.message().contains("layout.xml:3") && e.message().contains("<txt>"), "{}", e.message());
        let e = parse_layout("<widget><text value='a' colour='red'/></widget>").unwrap_err();
        assert!(e.message().contains("colour"), "{}", e.message());
    }

    #[test]
    fn layout_tree() {
        let n = parse_layout("<widget><row gap='4'><text value='{clock.time}'/><spacer/></row></widget>").unwrap();
        assert_eq!(n.tag, "row");
        assert_eq!(n.children.len(), 2);
        assert_eq!(n.children[0].attr("value"), Some("{clock.time}"));
    }

    #[test]
    fn style_classes() {
        let n = parse_layout(
            "<widget><style class='big' size='40' weight='700'/><style class='dim' color='secondary'/>\
             <text class='big dim' value='a' size='50'/></widget>",
        )
        .unwrap();
        assert_eq!((n.attr("size"), n.attr("weight"), n.attr("color")), (Some("50"), Some("700"), Some("secondary")));
        assert!(parse_layout("<widget><text class='nope' value='a'/></widget>").is_err());
        // A class attribute that does not fit the element is reported where it is used.
        let e = parse_layout("<widget><style class='t' value='x'/><rect class='t'/></widget>").unwrap_err();
        assert!(e.message().contains("<rect> has no attribute 'value'"), "{}", e.message());
    }

    #[test]
    fn states() {
        assert!(parse_layout("<widget><rect hover.fill='#fff' pressed.scale='0.95' action='go'/></widget>").is_ok());
        let e = parse_layout("<widget><rect hover.width='10'/></widget>").unwrap_err();
        assert!(e.message().contains("hover.width"), "{}", e.message());
        assert!(parse_layout("<widget><rect focus.fill='#fff'/></widget>").is_err());
        assert!(parse_layout("<widget><canvas/></widget>").is_err(), "canvas needs an id");
    }

    #[test]
    fn schema_file_is_current() {
        // Regenerate with: cargo run -p widget-engine --bin widget-dev -- --schema > schemas/layout.xsd
        let file = include_str!("../../../schemas/layout.xsd").replace("\r\n", "\n");
        assert_eq!(file, layout_schema(), "schemas/layout.xsd is stale");
    }

    #[test]
    fn versions() {
        assert!(check_engine_version("^0.1").is_ok());
        assert!(check_engine_version("^0.2").is_err());
        assert!(check_engine_version("^1.0").is_err());
    }
}
