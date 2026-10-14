//! Design tokens that are not colours.
//!
//! One corner radius for every rounded rectangle of the interface: tab chips, tool buttons and their cluster,
//! the new-tab button, flyouts, menus, the palette, the find bar, dialogs, settings cards, inputs, the
//! InfoBar and scrollbar thumbs. Nothing is a capsule or a circle, and a rectangle nested in another one keeps
//! the same radius instead of a computed smaller one.
//!
//! `CORNER_RADIUS` is the only place the number is written. `resources.xaml` receives it through
//! [`resources_xaml`] as `NkCornerRadius` and as the WinUI keys `ControlCornerRadius` and `OverlayCornerRadius`
//! (so system controls follow), and every template refers to `{StaticResource NkCornerRadius}`.
//! The test at the bottom keeps it that way.

/// Corner radius of every rounded rectangle, in DIP.
pub const CORNER_RADIUS: f64 = 8.0;

/// Placeholder of the radius in `resources.xaml`.
const RADIUS_PLACEHOLDER: &str = "@CORNER_RADIUS@";

const RESOURCES_XAML: &str = include_str!("xaml/resources.xaml");

/// The application resource dictionary: palette brushes, the button style and the radius resources.
pub fn resources_xaml() -> String {
    RESOURCES_XAML.replace(RADIUS_PLACEHOLDER, &CORNER_RADIUS.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;

    /// Every `.rs` and `.xaml` file of the application, as (path relative to `src`, contents).
    fn sources() -> Vec<(String, String)> {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in fs::read_dir(dir).expect("src is readable") {
                let path = entry.expect("directory entry").path();
                if path.is_dir() {
                    walk(&path, out);
                } else if matches!(path.extension().and_then(|e| e.to_str()), Some("rs" | "xaml")) {
                    out.push(path);
                }
            }
        }
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        walk(&src, &mut files);
        files
            .into_iter()
            .map(|path| {
                let name = path.strip_prefix(&src).expect("under src").to_string_lossy().replace('\\', "/");
                (name, fs::read_to_string(&path).expect("source is UTF-8"))
            })
            .collect()
    }

    /// Values of every `attribute="..."` in `text`.
    fn attribute_values<'a>(text: &'a str, attribute: &str) -> Vec<&'a str> {
        let needle = format!("{attribute}=\"");
        let mut values = Vec::new();
        let mut rest = text;
        while let Some(at) = rest.find(&needle) {
            let after = &rest[at + needle.len()..];
            let end = after.find('"').unwrap_or(after.len());
            values.push(&after[..end]);
            rest = &after[end..];
        }
        values
    }

    /// What a `CornerRadius` attribute may say: the token or the WinUI keys that carry it.
    const ALLOWED: [&str; 4] = [
        "{StaticResource NkCornerRadius}",
        "{ThemeResource ControlCornerRadius}",
        "{ThemeResource OverlayCornerRadius}",
        "{TemplateBinding CornerRadius}",
    ];

    #[test]
    fn markup_never_writes_a_radius_number() {
        for (name, text) in sources() {
            if name == "design.rs" {
                continue;
            }
            for value in attribute_values(&text, "CornerRadius") {
                assert!(ALLOWED.contains(&value), "{name}: CornerRadius=\"{value}\" must be one of {ALLOWED:?}");
            }
            for attribute in ["RadiusX", "RadiusY"] {
                assert!(
                    attribute_values(&text, attribute).is_empty(),
                    "{name}: {attribute} draws a rounded rectangle with its own radius; use a Border with the token"
                );
            }
        }
    }

    #[test]
    fn rust_code_sets_a_radius_only_from_the_token() {
        for (name, text) in sources().into_iter().filter(|(name, _)| name.ends_with(".rs") && name != "design.rs") {
            for (number, line) in text.lines().enumerate() {
                let sets_radius = line.contains("SetCornerRadius(") || line.contains("CornerRadius {");
                assert!(
                    !sets_radius || line.contains("CORNER_RADIUS"),
                    "{name}:{}: a radius must come from design::CORNER_RADIUS",
                    number + 1
                );
            }
        }
    }

    #[test]
    fn the_radius_resources_all_carry_the_token() {
        let resolved = resources_xaml();
        assert!(!resolved.contains(RADIUS_PLACEHOLDER), "the placeholder is replaced everywhere");
        let value = CORNER_RADIUS.to_string();
        for key in ["NkCornerRadius", "ControlCornerRadius", "OverlayCornerRadius"] {
            let element = format!("<CornerRadius x:Key=\"{key}\">{value}</CornerRadius>");
            assert!(resolved.contains(&element), "resources.xaml lacks {element}");
        }
        // A fourth CornerRadius resource with another number would be a second source of truth.
        assert_eq!(resolved.matches("<CornerRadius ").count(), 3);
    }

    #[test]
    fn chips_and_tools_use_the_token_and_the_documented_geometry() {
        let chip = sources().into_iter().find(|(name, _)| name == "chip.rs").expect("chip.rs").1;
        assert!(chip.contains("Height=\"32\""), "tab chips are 32 DIP high");
        assert!(chip.contains("CornerRadius=\"{StaticResource NkCornerRadius}\""));
        assert!(chip.contains("pub const CHIP_GAP: f64 = 4.0;"), "chips are 4 DIP apart");
    }
}
