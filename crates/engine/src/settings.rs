//! User customization: what the developer allows, what the user picked, the right-click menu.

use std::collections::BTreeMap;
use std::path::PathBuf;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Gdi::LOGFONTW;
use windows::Win32::UI::Controls::Dialogs::*;

use crate::i18n::t;
use crate::menu::Item;

/// What a widget lets end users change. Returned by [`crate::Widget::customization`].
#[derive(Clone, Debug, Default)]
pub struct Customization {
    /// Engine-drawn background under the widget's layers. None: the widget draws its own.
    pub card: Option<Card>,
    /// Which engine settings the user may change.
    pub allow: Allow,
    /// Default font. The user may change it when `allow.font` is set. Read it with [`crate::Scene::font`].
    pub font: Font,
    /// Widget-specific choices. Read them with [`crate::Scene::option`].
    pub options: Vec<Choice>,
}

impl Customization {
    /// False when the developer allows no changes: then right-click shows no menu at all.
    pub fn editable(&self) -> bool {
        let a = self.allow;
        a.size || a.background || a.theme || a.font || a.shape || a.colors || a.text || !self.options.is_empty()
    }
}

/// Engine-drawn widget background.
#[derive(Clone, Copy, Debug)]
pub struct Card {
    /// Corner radius in DIPs. Circular corners, the same for every background.
    pub radius: f32,
    pub background: Background,
    /// Hairline edge.
    pub border: bool,
    /// Soft drop shadow under the card. The widget window grows by the shadow's size.
    pub shadow: bool,
}

impl Default for Card {
    fn default() -> Self {
        Self { radius: 24.0, background: Background::Frosted, border: true, shadow: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Background {
    /// Opaque fill in the theme or tint color.
    Solid,
    /// Blurred, saturated wallpaper with a light tint, like frosted glass.
    Frosted,
    /// Only a faint tint. The wallpaper shows through.
    Clear,
    /// Liquid glass: the wallpaper, barely blurred, bends at the rounded edge like a lens,
    /// with a specular rim.
    Glass,
}

impl Background {
    fn key(self) -> &'static str {
        match self {
            Self::Solid => "solid",
            Self::Frosted => "frosted",
            Self::Clear => "clear",
            Self::Glass => "glass",
        }
    }

    pub(crate) fn parse(s: &str) -> Option<Self> {
        [Self::Solid, Self::Frosted, Self::Clear, Self::Glass].into_iter().find(|b| b.key() == s)
    }

    /// Drawn from the wallpaper under the widget: redraw while dragging.
    pub(crate) fn uses_wallpaper(self) -> bool {
        matches!(self, Self::Frosted | Self::Glass)
    }
}

/// Engine settings the user may change.
#[derive(Clone, Copy, Debug, Default)]
pub struct Allow {
    pub size: bool,
    /// Material, tint color, opacity. Needs a [`Card`].
    pub background: bool,
    pub theme: bool,
    /// Family, weight, width.
    pub font: bool,
    /// Corner radius, border, shadow. Needs a [`Card`].
    pub shape: bool,
    /// Accent color and text color.
    pub colors: bool,
    /// Text color only.
    pub text: bool,
}

impl Allow {
    pub const ALL: Self = Self { size: true, background: true, theme: true, font: true, shape: true, colors: true, text: true };
}

#[derive(Clone, Debug)]
pub struct Font {
    pub family: String,
    pub weight: DWRITE_FONT_WEIGHT,
    pub stretch: DWRITE_FONT_STRETCH,
}

impl Default for Font {
    fn default() -> Self {
        Self {
            family: "Nunito".into(),
            weight: DWRITE_FONT_WEIGHT_NORMAL,
            stretch: DWRITE_FONT_STRETCH_NORMAL,
        }
    }
}

/// A widget-specific setting with a fixed set of values, shown as a submenu.
#[derive(Clone, Debug)]
pub struct Choice {
    pub key: String,
    pub label: String,
    /// (value, label) pairs. Empty for a path.
    pub values: Vec<(String, String)>,
    pub default: String,
    pub kind: ChoiceKind,
    /// Values that give the widget another size: (value, (width, height)).
    pub sizes: Vec<(String, (f32, f32))>,
}

/// What a [`Choice`] holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChoiceKind {
    /// One of `values`.
    #[default]
    List,
    /// The path of a picture file the user picks, "" for none.
    Image,
    /// The path of a folder the user picks, "" for none.
    Folder,
}

/// On/off choice with values "on" and "off".
pub fn toggle(key: &str, label: &str, on_by_default: bool) -> Choice {
    Choice {
        key: key.into(),
        label: label.into(),
        values: vec![("on".into(), "Show".into()), ("off".into(), "Hide".into())],
        default: if on_by_default { "on" } else { "off" }.into(),
        kind: ChoiceKind::List,
        sizes: Vec::new(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ThemeMode {
    System,
    Light,
    Dark,
}

/// Settings after applying the user's picks to the developer's defaults.
#[derive(Clone, Debug)]
pub(crate) struct Resolved {
    pub size: f32,
    pub card: Option<Card>,
    pub opacity: f32,
    /// Liquid glass blur, Gaussian sigma in DIPs.
    pub blur: f32,
    /// Liquid glass tint: alpha of a veil over the glass, dark in dark mode, light in light
    /// mode (or the tint color).
    pub veil: f32,
    /// 0xRRGGBB.
    pub tint: Option<u32>,
    /// 0xRRGGBB overrides of the system accent color and the text color.
    pub accent: Option<u32>,
    pub text: Option<u32>,
    pub theme: ThemeMode,
    pub font: Font,
    pub options: BTreeMap<String, String>,
}

pub(crate) fn resolve(c: &Customization, user: &BTreeMap<String, String>) -> Resolved {
    let get = |k: &str| user.get(k).map(String::as_str);
    let a = c.allow;
    let mut card = c.card;
    let mut font = c.font.clone();
    let on = |k: &str| get(k).map(|v| v == "on");
    if let Some(card) = card.as_mut() {
        if a.background {
            if let Some(b) = get("background").and_then(Background::parse) {
                card.background = b;
            }
        }
        if a.shape {
            if let Some(r) = get("radius").and_then(|v| v.parse::<f32>().ok()) {
                card.radius = r.clamp(0.0, 64.0);
            }
            card.border = on("border").unwrap_or(card.border);
            card.shadow = on("shadow").unwrap_or(card.shadow);
        }
    }
    let hex = |k: &str| get(k).filter(|_| a.colors).and_then(|v| u32::from_str_radix(v, 16).ok());
    if a.font {
        if let Some(f) = get("font").filter(|f| !f.is_empty()) {
            font.family = f.to_string();
        }
        if let Some(w) = get("weight").and_then(|v| v.parse().ok()) {
            font.weight = DWRITE_FONT_WEIGHT(w);
        }
        if let Some(s) = get("width").and_then(|v| v.parse().ok()) {
            font.stretch = DWRITE_FONT_STRETCH(s);
        }
    }
    let options = c
        .options
        .iter()
        .map(|o| {
            let v = get(&format!("opt.{}", o.key)).filter(|v| o.kind != ChoiceKind::List || o.values.iter().any(|(x, _)| x == v));
            (o.key.clone(), v.unwrap_or(&o.default).to_string())
        })
        .collect();
    Resolved {
        size: get("size").filter(|_| a.size).and_then(|v| v.parse().ok()).unwrap_or(1.0f32).clamp(0.5, 3.0),
        card,
        opacity: get("opacity").filter(|_| a.background).and_then(|v| v.parse().ok()).unwrap_or(1.0f32).clamp(0.1, 1.0),
        blur: get("blur").filter(|_| a.background).and_then(|v| v.parse().ok()).unwrap_or(GLASS_BLUR).clamp(0.0, 30.0),
        veil: get("veil").filter(|_| a.background).and_then(|v| v.parse().ok()).unwrap_or(GLASS_VEIL).clamp(0.0, 0.8),
        tint: get("tint").filter(|_| a.background).and_then(|v| u32::from_str_radix(v, 16).ok()),
        accent: hex("accent"),
        text: get("text").filter(|_| a.colors || a.text).and_then(|v| u32::from_str_radix(v, 16).ok()),
        theme: match get("theme").filter(|_| a.theme) {
            Some("light") => ThemeMode::Light,
            Some("dark") => ThemeMode::Dark,
            _ => ThemeMode::System,
        },
        font,
        options,
    }
}

// --- persistence -----------------------------------------------------------

/// `%LOCALAPPDATA%\<exe name>\widgets.ini`, lines of `instance.key=value`.
pub(crate) struct Store {
    path: Option<PathBuf>,
    /// instance id -> key -> value
    pub data: BTreeMap<String, BTreeMap<String, String>>,
}

impl Store {
    pub fn load() -> Self {
        let path = (|| {
            let base = std::env::var_os("LOCALAPPDATA")?;
            let app = std::env::current_exe().ok()?.file_stem()?.to_owned();
            Some(PathBuf::from(base).join(app).join("widgets.ini"))
        })();
        let text = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        Self { path, data: parse(&text) }
    }

    pub fn get(&self, id: &str) -> BTreeMap<String, String> {
        self.data.get(id).cloned().unwrap_or_default()
    }

    pub fn set(&mut self, id: &str, key: &str, value: &str) {
        self.data.entry(id.into()).or_default().insert(key.into(), value.into());
        self.save();
    }

    /// Forgets an instance: settings and position.
    pub fn remove(&mut self, id: &str) {
        self.data.remove(id);
        self.save();
    }

    /// Drops all settings of an instance except its position.
    pub fn reset(&mut self, id: &str) {
        if let Some(m) = self.data.get_mut(id) {
            m.retain(|k, _| k == "x" || k == "y");
        }
        self.save();
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(path, serialize(&self.data)) {
            eprintln!("widget engine: cannot save {}: {e}", path.display());
        }
    }
}

fn parse(text: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let Some((id, key)) = k.trim().split_once('.') else { continue };
        out.entry(id.into()).or_default().insert(key.into(), v.trim().into());
    }
    out
}

fn serialize(data: &BTreeMap<String, BTreeMap<String, String>>) -> String {
    let mut s = String::new();
    for (id, m) in data {
        for (k, v) in m {
            // Values never contain newlines: they come from menus and dialogs.
            s.push_str(&format!("{id}.{k}={}\n", v.replace(['\r', '\n'], " ")));
        }
    }
    s
}

// --- right-click menu ------------------------------------------------------

/// What the user picked in the menu.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Action {
    Set(String, String),
    PickFont,
    /// Color dialog for a 0xRRGGBB setting: tint, accent, text.
    PickColor(String),
    /// File or folder dialog for a path option (`opt.<key>`).
    PickPath(String, ChoiceKind),
    Reset,
    /// Removes the widget and its saved settings.
    Delete,
    /// An app menu item, see [`crate::popup_menu`].
    App(String),
}

/// Default liquid glass blur in DIPs: a light frost, like Apple's.
const GLASS_BLUR: f32 = 6.0;
/// Default liquid glass tint: a small one, so text stays readable on busy wallpapers.
const GLASS_VEIL: f32 = 0.18;

/// Background colors to pick from: (label key, 0xRRGGBB).
const COLORS: &[(&str, &str)] = &[
    ("Graphite", "2C2C2E"),
    ("Midnight", "1E2A47"),
    ("Ocean", "1565C0"),
    ("Teal", "0F766E"),
    ("Forest", "2E5E3E"),
    ("Plum", "5B2C6F"),
    ("Berry", "A3245B"),
    ("Coral", "FF6F61"),
    ("Amber", "F6B73C"),
    ("Sand", "EADFCB"),
    ("Mint", "CDEFE1"),
    ("Sky", "D3E7FF"),
    ("Lavender", "E4DBF8"),
    ("Blush", "F9D8DF"),
];

/// Text colors to pick from: (label key, 0xRRGGBB).
const TEXT_COLORS: &[(&str, &str)] = &[
    ("White|color", "FFFFFF"),
    ("Black|color", "000000"),
    ("Graphite", "2C2C2E"),
    ("Ocean", "1565C0"),
    ("Teal", "0F766E"),
    ("Berry", "A3245B"),
    ("Coral", "FF6F61"),
    ("Amber", "F6B73C"),
    ("Sand", "EADFCB"),
    ("Mint", "CDEFE1"),
    ("Sky", "D3E7FF"),
    ("Lavender", "E4DBF8"),
    ("Blush", "F9D8DF"),
];

/// Submenu that sets `key` to one of `values`, (value, label key) pairs.
fn pick(label: &str, key: &str, current: &str, values: &[(&str, &str)]) -> Item {
    let one = |v: &str, l: &str| Item::pick(t(l), v == current, Action::Set(key.into(), v.into()));
    let mut items = Vec::new();
    // Long lists come in groups: the value "Europe/Paris" goes in the submenu "Europe".
    let mut groups: Vec<(&str, Vec<Item>)> = Vec::new();
    for (v, l) in values {
        match v.split_once('/') {
            Some((g, _)) => match groups.iter_mut().find(|(n, _)| *n == g) {
                Some((_, list)) => list.push(one(v, l)),
                None => groups.push((g, vec![one(v, l)])),
            },
            None => items.push(one(v, l)),
        }
    }
    items.extend(groups.into_iter().map(|(g, list)| Item::sub(t(g), list)));
    Item::sub(t(label), items)
}

/// The widget's size: the package's, unless the picked value of an option sets its own
/// (a clock without its dial is square).
pub(crate) fn size(c: &Customization, options: &BTreeMap<String, String>, base: (f32, f32)) -> (f32, f32) {
    c.options
        .iter()
        .find_map(|o| options.get(&o.key).and_then(|v| o.sizes.iter().find(|(k, _)| k == v)).map(|(_, s)| *s))
        .unwrap_or(base)
}

/// Submenu with preset colors, the color dialog and a reset to the default.
fn color(label: &str, key: &str, current: Option<u32>, presets: &[(&str, &str)]) -> Item {
    let cur = current.map(|c| format!("{c:06X}"));
    let mut items: Vec<Item> =
        presets.iter().map(|(l, v)| Item::pick(t(l), cur.as_deref() == Some(*v), Action::Set(key.into(), (*v).into()))).collect();
    let custom = cur.as_deref().is_some_and(|c| !presets.iter().any(|(_, v)| *v == c));
    items.push(Item::pick(t("Choose color…"), custom, Action::PickColor(key.into())));
    items.push(Item::pick(t("Default"), cur.is_none(), Action::Set(key.into(), String::new())));
    Item::sub(t(label), items)
}

/// The right-click menu: Delete widget, then Edit (what the package allows).
/// None when the developer allows no changes.
pub(crate) fn menu_items(c: &Customization, r: &Resolved) -> Option<Vec<Item>> {
    if !c.editable() {
        return None;
    }
    let a = c.allow;
    let mut edit = Vec::new();
    if a.size {
        let cur = format!("{}", r.size);
        edit.push(pick(
            "Size",
            "size",
            &cur,
            &[("0.75", "Small"), ("1", "Normal|size"), ("1.25", "Large"), ("1.5", "Extra large"), ("2", "Huge")],
        ));
    }
    if let (true, Some(card)) = (a.background, r.card) {
        edit.push(pick(
            "Background",
            "background",
            card.background.key(),
            &[("frosted", "Frosted glass"), ("glass", "Liquid glass"), ("solid", "Solid"), ("clear", "Transparent")],
        ));
        let cur = format!("{}", r.opacity);
        edit.push(pick(
            "Transparency",
            "opacity",
            &cur,
            &[("1", "None|transparency"), ("0.85", "Low"), ("0.6", "Medium"), ("0.35", "High")],
        ));
        if card.background == Background::Glass {
            let cur = format!("{}", r.blur);
            edit.push(pick(
                "Blur",
                "blur",
                &cur,
                &[("0", "None|blur"), ("2", "Low|blur"), ("6", "Medium|blur"), ("14", "High|blur")],
            ));
            let cur = format!("{}", r.veil);
            edit.push(pick(
                "Glass tint",
                "veil",
                &cur,
                &[("0", "None|tint"), ("0.18", "Small|tint"), ("0.32", "Medium|tint"), ("0.5", "Strong|tint")],
            ));
        }
        edit.push(color("Background color", "tint", r.tint, COLORS));
    }
    if let (true, Some(card)) = (a.shape, r.card) {
        let cur = format!("{}", card.radius);
        let def = format!("{}", c.card.map_or(24.0, |c| c.radius));
        let mut corners = vec![("0", "Square"), ("12", "Slightly rounded"), ("20", "Rounded"), ("32", "Very rounded")];
        if !corners.iter().any(|(v, _)| *v == def) {
            corners.push((&def, "Default"));
        }
        edit.push(pick("Corners", "radius", &cur, &corners));
        let onoff = |b: bool| if b { "on" } else { "off" };
        edit.push(pick("Border", "border", onoff(card.border), &[("on", "Show"), ("off", "Hide")]));
        edit.push(pick("Shadow", "shadow", onoff(card.shadow), &[("on", "Show"), ("off", "Hide")]));
    }
    if a.theme {
        let cur = match r.theme {
            ThemeMode::System => "system",
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        };
        edit.push(pick("Theme", "theme", cur, &[("system", "Match system"), ("light", "Light|theme"), ("dark", "Dark")]));
    }
    if a.colors {
        edit.push(color("Accent color", "accent", r.accent, &[]));
    }
    if a.colors || a.text {
        edit.push(color("Text color", "text", r.text, TEXT_COLORS));
    }
    if a.font {
        // The package's font, the shipped ones by style, and any installed font.
        let default = r.font.family == c.font.family;
        let shipped = |f: &str| crate::fonts::GROUPS.iter().any(|(_, g)| g.contains(&f));
        let mut fonts = vec![Item::pick(t("Default"), default, Action::Set("font".into(), String::new()))];
        for (group, families) in crate::fonts::GROUPS {
            let items = families.iter().map(|f| Item::pick(f, *f == r.font.family, Action::Set("font".into(), (*f).into()))).collect();
            fonts.push(Item::sub(t(group), items));
        }
        fonts.push(Item::pick(t("More fonts…"), !default && !shipped(&r.font.family), Action::PickFont));
        edit.push(Item::sub(t("Font"), fonts));
        let w = r.font.weight.0.to_string();
        edit.push(pick(
            "Weight",
            "weight",
            &w,
            &[("300", "Light|weight"), ("400", "Regular"), ("600", "Semibold"), ("700", "Bold"), ("900", "Black")],
        ));
        let s = r.font.stretch.0.to_string();
        edit.push(pick(
            "Width",
            "width",
            &s,
            &[("3", "Condensed"), ("4", "Semi-condensed"), ("5", "Normal|width"), ("7", "Wide")],
        ));
    }
    for o in &c.options {
        let cur = r.options.get(&o.key).cloned().unwrap_or_default();
        if o.kind != ChoiceKind::List {
            let key = format!("opt.{}", o.key);
            let choose = if o.kind == ChoiceKind::Image { "Choose photo…" } else { "Choose folder…" };
            let items = vec![
                Item::pick(t(choose), !cur.is_empty(), Action::PickPath(key.clone(), o.kind)),
                Item::pick(t("None|path"), cur.is_empty(), Action::Set(key, String::new())),
            ];
            edit.push(Item::sub(&o.label, items));
            continue;
        }
        let values: Vec<(&str, &str)> = o.values.iter().map(|(v, l)| (v.as_str(), l.as_str())).collect();
        edit.push(pick(&o.label, &format!("opt.{}", o.key), &cur, &values));
    }
    edit.push(Item::pick(t("Reset to defaults"), false, Action::Reset));
    let delete = Item { danger: true, ..Item::pick(t("Delete widget"), false, Action::Delete) };
    Some(vec![delete, Item::sub(t("Edit"), edit)])
}

/// System font dialog. Returns (family, weight).
pub(crate) fn pick_font(owner: HWND, current: &Font) -> Option<(String, i32)> {
    let mut lf = LOGFONTW { lfWeight: current.weight.0, ..Default::default() };
    for (d, s) in lf.lfFaceName.iter_mut().zip(current.family.encode_utf16().take(31)) {
        *d = s;
    }
    let mut cf = CHOOSEFONTW {
        lStructSize: size_of::<CHOOSEFONTW>() as u32,
        hwndOwner: owner,
        lpLogFont: &mut lf,
        Flags: CF_SCREENFONTS | CF_INITTOLOGFONTSTRUCT | CF_NOSCRIPTSEL | CF_FORCEFONTEXIST,
        ..Default::default()
    };
    if !unsafe { ChooseFontW(&mut cf) }.as_bool() {
        return None;
    }
    let len = lf.lfFaceName.iter().position(|&c| c == 0).unwrap_or(32);
    Some((String::from_utf16_lossy(&lf.lfFaceName[..len]), lf.lfWeight))
}

/// File dialog for a picture, or a folder dialog. Returns the path.
pub(crate) fn pick_path(owner: HWND, kind: ChoiceKind) -> Option<String> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::*;
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let mut options = dialog.GetOptions().ok()? | FOS_FORCEFILESYSTEM;
        if kind == ChoiceKind::Folder {
            options |= FOS_PICKFOLDERS;
        } else {
            let name = windows::core::HSTRING::from(t("Pictures|files"));
            let filter = [COMDLG_FILTERSPEC {
                pszName: windows::core::PCWSTR(name.as_ptr()),
                pszSpec: windows::core::w!("*.jpg;*.jpeg;*.png;*.bmp;*.gif;*.webp;*.heic;*.tif;*.tiff"),
            }];
            dialog.SetFileTypes(&filter).ok()?;
        }
        dialog.SetOptions(options).ok()?;
        dialog.Show(Some(owner)).ok()?;
        let item = dialog.GetResult().ok()?;
        let path = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = path.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(path.0 as _));
        s
    }
}

/// System color dialog. Returns 0xRRGGBB.
pub(crate) fn pick_color(owner: HWND, current: Option<u32>) -> Option<u32> {
    static mut CUSTOM: [COLORREF; 16] = [COLORREF(0x00FF_FFFF); 16];
    let rgb = current.unwrap_or(0x808080);
    let mut cc = CHOOSECOLORW {
        lStructSize: size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: owner,
        rgbResult: COLORREF(((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF)),
        // SAFETY: only touched on the UI thread while the modal dialog runs.
        lpCustColors: &raw mut CUSTOM as *mut COLORREF,
        Flags: CC_RGBINIT | CC_FULLOPEN,
        ..Default::default()
    };
    if !unsafe { ChooseColorW(&mut cc) }.as_bool() {
        return None;
    }
    let c = cc.rgbResult.0;
    Some(((c & 0xFF) << 16) | (c & 0xFF00) | ((c >> 16) & 0xFF))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini_roundtrip() {
        let text = "clock.size=1.25\nclock.font=Bahnschrift\nother.x=10\nbad line\n";
        let data = parse(text);
        assert_eq!(data["clock"]["size"], "1.25");
        assert_eq!(data["other"]["x"], "10");
        assert_eq!(parse(&serialize(&data)), data);
    }

    #[test]
    fn option_sizes_and_groups() {
        let mut dial = toggle("dial", "Dial", true);
        dial.sizes = vec![("off".into(), (170.0, 170.0))];
        let c = Customization { options: vec![dial], ..Default::default() };
        let opts = |v: &str| BTreeMap::from([("dial".to_string(), v.to_string())]);
        assert_eq!(size(&c, &opts("on"), (352.0, 170.0)), (352.0, 170.0));
        assert_eq!(size(&c, &opts("off"), (352.0, 170.0)), (170.0, 170.0));
        let menu = pick("City", "opt.city", "Europe/Paris", &[("auto", "Automatic"), ("Europe/Paris", "Paris"), ("Asia/Tokyo", "Tokyo"), ("Europe/Rome", "Rome")]);
        // Automatic, then the Europe and Asia submenus.
        let sub = |i: &Item| match &i.kind {
            crate::menu::Kind::Sub(v) => v.clone(),
            _ => Vec::new(),
        };
        let top = sub(&menu);
        assert_eq!(top.len(), 3);
        assert_eq!(sub(&top[1]).len(), 2);
        assert!(sub(&top[1])[0].checked);
    }

    #[test]
    fn resolve_respects_allow() {
        let user: BTreeMap<String, String> =
            [("size", "2"), ("font", "Georgia"), ("opt.date", "off"), ("opt.bogus", "x")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
        let mut c = Customization { options: vec![toggle("date", "Date", true)], ..Default::default() };
        let r = resolve(&c, &user);
        assert_eq!(r.size, 1.0, "size not allowed");
        assert_eq!(r.font.family, "Nunito", "font not allowed");
        assert_eq!(r.options["date"], "off", "widget options are always user-editable");
        c.allow = Allow::ALL;
        let r = resolve(&c, &user);
        assert_eq!(r.size, 2.0);
        assert_eq!(r.font.family, "Georgia");
    }
}
