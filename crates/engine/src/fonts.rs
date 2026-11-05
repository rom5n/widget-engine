//! Fonts shipped with the app: every .ttf/.otf in a `fonts` folder next to the exe (or the
//! repository's, for builds in `target`), usable by family name like installed fonts.

use std::cell::OnceCell;
use std::path::PathBuf;

use windows::Win32::Graphics::DirectWrite::*;
use windows::core::{HSTRING, Interface, Result};

/// Font menu groups: (label key, families). All ship in the repository's `fonts` folder.
pub(crate) const GROUPS: &[(&str, &[&str])] = &[
    ("Rounded|font", &["Nunito", "Comfortaa", "Rubik"]),
    ("Modern|font", &["Manrope", "Onest", "Geologica", "Montserrat", "Unbounded"]),
    ("Sharp|font", &["Oswald", "Russo One", "Tektur"]),
    ("Techno|font", &["Exo 2", "Jura"]),
    ("Monospace|font", &["JetBrains Mono"]),
];

fn dirs() -> Vec<PathBuf> {
    let exe = std::env::current_exe().unwrap_or_default();
    let dir = exe.parent().map(PathBuf::from).unwrap_or_default();
    vec![dir.join("fonts"), dir.join("../../fonts")]
}

thread_local! {
    static COLLECTION: OnceCell<Option<IDWriteFontCollection>> = const { OnceCell::new() };
}

/// The system fonts plus the shipped ones. None: no shipped fonts (use the system collection).
pub(crate) fn collection(dw: &IDWriteFactory) -> Option<IDWriteFontCollection> {
    COLLECTION.with(|c| c.get_or_init(|| build(dw).map_err(|e| eprintln!("widget engine: fonts: {e}")).ok().flatten()).clone())
}

fn build(dw: &IDWriteFactory) -> Result<Option<IDWriteFontCollection>> {
    let files: Vec<PathBuf> = dirs()
        .iter()
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ttf") || x.eq_ignore_ascii_case("otf")))
        .collect();
    if files.is_empty() {
        return Ok(None);
    }
    unsafe {
        let f5: IDWriteFactory5 = dw.cast()?;
        let builder = f5.CreateFontSetBuilder()?;
        for p in &files {
            let file = dw.CreateFontFileReference(&HSTRING::from(p.as_os_str()), None)?;
            // Variable fonts add their named instances (Light, Bold, ...).
            builder.AddFontFile(&file)?;
        }
        builder.AddFontSet(&f5.GetSystemFontSet()?)?;
        let set = builder.CreateFontSet()?;
        Ok(Some(f5.CreateFontCollectionFromFontSet(&set)?.cast()?))
    }
}

/// `CreateTextFormat` that also finds the shipped fonts.
pub(crate) fn text_format(
    dw: &IDWriteFactory,
    family: &str,
    weight: DWRITE_FONT_WEIGHT,
    stretch: DWRITE_FONT_STRETCH,
    size: f32,
) -> Result<IDWriteTextFormat> {
    let collection = collection(dw);
    unsafe { dw.CreateTextFormat(&HSTRING::from(family), collection.as_ref(), weight, DWRITE_FONT_STYLE_NORMAL, stretch, size, windows::core::w!("")) }
}
