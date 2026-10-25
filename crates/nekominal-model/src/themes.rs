//! Terminal color schemes: the built-in ones, and those of `settings.json` (`schemes`), which come after them.

use std::borrow::Cow;

use serde::ser::{Serialize, SerializeMap, Serializer};
use serde_json::{Map, Value};

use crate::settings::Expected;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    /// `0xRRGGBB`, fully opaque.
    pub const fn rgb(value: u32) -> Self {
        Self { r: (value >> 16) as u8, g: (value >> 8) as u8, b: value as u8, a: 255 }
    }

    /// `0xRRGGBBAA`.
    pub const fn rgba(value: u32) -> Self {
        Self { r: (value >> 24) as u8, g: (value >> 16) as u8, b: (value >> 8) as u8, a: value as u8 }
    }

    /// Parses `#rgb`, `#rrggbb` or `#rrggbbaa`.
    pub fn parse_hex(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('#')?;
        if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        match digits.len() {
            3 => u32::from_str_radix(&digits.chars().flat_map(|c| [c, c]).collect::<String>(), 16).ok().map(Self::rgb),
            6 => u32::from_str_radix(digits, 16).ok().map(Self::rgb),
            8 => u32::from_str_radix(digits, 16).ok().map(Self::rgba),
            _ => None,
        }
    }

    /// Linear interpolation of the color channels (alpha stays opaque).
    pub fn mix(self, other: Self, amount: f32) -> Self {
        let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
        Self { r: channel(self.r, other.r), g: channel(self.g, other.g), b: channel(self.b, other.b), a: 255 }
    }

    /// Relative luminance in 0..1 (Rec. 709 weights on the encoded values, as in the previous UI).
    pub fn luminance(self) -> f32 {
        0.2126 * self.r as f32 / 255.0 + 0.7152 * self.g as f32 / 255.0 + 0.0722 * self.b as f32 / 255.0
    }

    pub fn is_light(self) -> bool {
        self.luminance() > 0.5
    }

    /// `#rrggbb`, or `#rrggbbaa` when it is not opaque: the reverse of [`Color::parse_hex`].
    pub fn to_hex(self) -> String {
        let rgb = format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b);
        if self.a == 255 { rgb } else { format!("{rgb}{:02x}", self.a) }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scheme {
    /// Borrowed for the built-in schemes, owned for those of `settings.json`.
    pub name: Cow<'static, str>,
    pub background: Color,
    pub foreground: Color,
    pub cursor: Color,
    pub selection: Color,
    /// black, red, green, yellow, blue, magenta, cyan, white, then the bright variants.
    pub ansi: [Color; 16],
}

const fn ansi(values: [u32; 16]) -> [Color; 16] {
    let mut out = [Color::rgb(0); 16];
    let mut i = 0;
    while i < 16 {
        out[i] = Color::rgb(values[i]);
        i += 1;
    }
    out
}

pub const DEFAULT_THEME: &str = "Tokyo Night";

pub static SCHEMES: [Scheme; 10] = [
    Scheme {
        name: Cow::Borrowed("Tokyo Night"),
        background: Color::rgb(0x1a1b26),
        foreground: Color::rgb(0xc0caf5),
        cursor: Color::rgb(0xc0caf5),
        selection: Color::rgb(0x33467c),
        ansi: ansi([
            0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6, 0x414868, 0xf7768e, 0x9ece6a,
            0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
        ]),
    },
    Scheme {
        name: Cow::Borrowed("Campbell"),
        background: Color::rgb(0x0c0c0c),
        foreground: Color::rgb(0xcccccc),
        cursor: Color::rgb(0xcccccc),
        selection: Color::rgba(0xffffff40),
        ansi: ansi([
            0x0c0c0c, 0xc50f1f, 0x13a10e, 0xc19c00, 0x0037da, 0x881798, 0x3a96dd, 0xcccccc, 0x767676, 0xe74856, 0x16c60c,
            0xf9f1a5, 0x3b78ff, 0xb4009e, 0x61d6d6, 0xf2f2f2,
        ]),
    },
    Scheme {
        name: Cow::Borrowed("One Half Dark"),
        background: Color::rgb(0x282c34),
        foreground: Color::rgb(0xdcdfe4),
        cursor: Color::rgb(0xdcdfe4),
        selection: Color::rgb(0x474e5d),
        ansi: ansi([
            0x282c34, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xdcdfe4, 0x5a6374, 0xe06c75, 0x98c379,
            0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xdcdfe4,
        ]),
    },
    Scheme {
        name: Cow::Borrowed("Catppuccin Mocha"),
        background: Color::rgb(0x1e1e2e),
        foreground: Color::rgb(0xcdd6f4),
        cursor: Color::rgb(0xf5e0dc),
        selection: Color::rgb(0x585b70),
        ansi: ansi([
            0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de, 0x585b70, 0xf38ba8, 0xa6e3a1,
            0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
        ]),
    },
    Scheme {
        name: Cow::Borrowed("Dracula"),
        background: Color::rgb(0x282a36),
        foreground: Color::rgb(0xf8f8f2),
        cursor: Color::rgb(0xf8f8f2),
        selection: Color::rgb(0x44475a),
        ansi: ansi([
            0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2, 0x6272a4, 0xff6e6e, 0x69ff94,
            0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff,
        ]),
    },
    Scheme {
        name: Cow::Borrowed("Nord"),
        background: Color::rgb(0x2e3440),
        foreground: Color::rgb(0xd8dee9),
        cursor: Color::rgb(0xd8dee9),
        selection: Color::rgb(0x434c5e),
        ansi: ansi([
            0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0, 0x4c566a, 0xbf616a, 0xa3be8c,
            0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4,
        ]),
    },
    Scheme {
        name: Cow::Borrowed("Gruvbox Dark"),
        background: Color::rgb(0x282828),
        foreground: Color::rgb(0xebdbb2),
        cursor: Color::rgb(0xebdbb2),
        selection: Color::rgb(0x504945),
        ansi: ansi([
            0x282828, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984, 0x928374, 0xfb4934, 0xb8bb26,
            0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xebdbb2,
        ]),
    },
    Scheme {
        name: Cow::Borrowed("Solarized Dark"),
        background: Color::rgb(0x002b36),
        foreground: Color::rgb(0x839496),
        cursor: Color::rgb(0x839496),
        selection: Color::rgb(0x073642),
        ansi: ansi([
            0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5, 0x002b36, 0xcb4b16, 0x586e75,
            0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
        ]),
    },
    Scheme {
        name: Cow::Borrowed("One Half Light"),
        background: Color::rgb(0xfafafa),
        foreground: Color::rgb(0x383a42),
        cursor: Color::rgb(0x383a42),
        selection: Color::rgb(0xbfceff),
        ansi: ansi([
            0x383a42, 0xe45649, 0x50a14f, 0xc18401, 0x0184bc, 0xa626a4, 0x0997b3, 0xfafafa, 0x4f525d, 0xdf6c75, 0x98c379,
            0xe4c07a, 0x61afef, 0xc577dd, 0x56b5c1, 0xffffff,
        ]),
    },
    Scheme {
        name: Cow::Borrowed("Solarized Light"),
        background: Color::rgb(0xfdf6e3),
        foreground: Color::rgb(0x657b83),
        cursor: Color::rgb(0x657b83),
        selection: Color::rgb(0xeee8d5),
        ansi: ansi([
            0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5, 0x002b36, 0xcb4b16, 0x586e75,
            0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
        ]),
    },
];

/// The names of the built-in schemes, then those of `custom` (the `schemes` of the settings). One of `custom` with a
/// built-in name takes the place of that one rather than coming twice.
pub fn theme_names(custom: &[Scheme]) -> impl Iterator<Item = &str> {
    let builtin = |name: &str| SCHEMES.iter().any(|scheme| scheme.name == name);
    SCHEMES.iter().map(|scheme| &*scheme.name).chain(custom.iter().map(|scheme| &*scheme.name).filter(move |name| !builtin(name)))
}

/// The scheme called `name` among `custom` and then the built-in ones, or the default one when the name is unknown.
pub fn find<'a>(custom: &'a [Scheme], name: &str) -> &'a Scheme {
    let named = |name: &str| custom.iter().chain(&SCHEMES).find(|scheme| scheme.name == name);
    named(name).or_else(|| named(DEFAULT_THEME)).expect("default theme exists")
}

/// The built-in scheme called `name`, or the default one when the name is unknown.
pub fn scheme(name: &str) -> &'static Scheme {
    find(&[], name)
}

pub fn is_known(custom: &[Scheme], name: &str) -> bool {
    custom.iter().chain(&SCHEMES).any(|scheme| scheme.name == name)
}

/// The keys Windows Terminal gives the 16 colours of a scheme, in the order of [`Scheme::ansi`].
const ANSI_KEYS: [&str; 16] = [
    "black", "red", "green", "yellow", "blue", "purple", "cyan", "white", "brightBlack", "brightRed", "brightGreen", "brightYellow", "brightBlue",
    "brightPurple", "brightCyan", "brightWhite",
];

impl Scheme {
    /// A scheme written as Windows Terminal writes one (`name`, `background`, `foreground`, `cursorColor`,
    /// `selectionBackground`, `black` … `brightWhite`), so that one copied from its settings works as is. `magenta`
    /// stands for `purple`, as some scheme sites write it. The error is the key that is missing or not a colour.
    pub fn from_json(map: &Map<String, Value>) -> Result<Scheme, (String, Expected)> {
        let name = match map.get("name") {
            Some(Value::String(name)) if !name.trim().is_empty() => name.trim().to_owned(),
            Some(Value::String(_)) | None => return Err(("name".to_owned(), Expected::Required)),
            Some(_) => return Err(("name".to_owned(), Expected::Text)),
        };
        let color = |key: &str| match map.get(key) {
            None => Ok(None),
            Some(value) => value.as_str().and_then(Color::parse_hex).map(Some).ok_or_else(|| (key.to_owned(), Expected::Color)),
        };
        let required = |key: &str| color(key)?.ok_or_else(|| (key.to_owned(), Expected::Required));
        let background = required("background")?;
        let foreground = required("foreground")?;
        let mut ansi = [Color::rgb(0); 16];
        for (slot, key) in ansi.iter_mut().zip(ANSI_KEYS) {
            let alias = key.replace("purple", "magenta").replace("Purple", "Magenta");
            *slot = required(if map.contains_key(key) || !map.contains_key(&alias) { key } else { &alias })?;
        }
        Ok(Scheme {
            name: Cow::Owned(name),
            background,
            foreground,
            cursor: color("cursorColor")?.unwrap_or(foreground),
            // A quarter of the text colour over the background reads on a light scheme as on a dark one.
            selection: color("selectionBackground")?.unwrap_or(Color { a: 0x40, ..foreground }),
            ansi,
        })
    }
}

/// Written back with the keys [`Scheme::from_json`] reads.
impl Serialize for Scheme {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(5 + ANSI_KEYS.len()))?;
        map.serialize_entry("name", &self.name)?;
        map.serialize_entry("background", &self.background.to_hex())?;
        map.serialize_entry("foreground", &self.foreground.to_hex())?;
        map.serialize_entry("cursorColor", &self.cursor.to_hex())?;
        map.serialize_entry("selectionBackground", &self.selection.to_hex())?;
        for (key, color) in ANSI_KEYS.iter().zip(self.ansi) {
            map.serialize_entry(key, &color.to_hex())?;
        }
        map.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_names_are_unique_and_include_the_default() {
        let names: Vec<_> = theme_names(&[]).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
        assert!(names.contains(&DEFAULT_THEME));
    }

    #[test]
    fn unknown_names_fall_back_to_the_default() {
        assert_eq!(scheme("nope").name, DEFAULT_THEME);
        assert_eq!(scheme("Nord").name, "Nord");
        assert!(is_known(&[], "Dracula"));
        assert!(!is_known(&[], "nope"));
    }

    fn custom(name: &str, background: u32) -> Scheme {
        Scheme { name: Cow::Owned(name.to_owned()), background: Color::rgb(background), ..scheme("Nord").clone() }
    }

    #[test]
    fn custom_schemes_come_after_the_built_in_ones_and_replace_one_of_their_name() {
        let mine = [custom("Mine", 0x101010), custom("Nord", 0x202020)];
        let names: Vec<_> = theme_names(&mine).collect();
        assert_eq!(names.len(), SCHEMES.len() + 1, "the Nord of the settings is not listed twice");
        assert_eq!(names.last(), Some(&"Mine"));
        assert_eq!(find(&mine, "Mine").background, Color::rgb(0x101010));
        assert_eq!(find(&mine, "Nord").background, Color::rgb(0x202020));
        assert_eq!(find(&mine, "Dracula").name, "Dracula");
        assert_eq!(find(&mine, "nope").name, DEFAULT_THEME, "an unknown name still falls back to the default");
        assert!(is_known(&mine, "Mine"));
        assert!(!is_known(&[], "Mine"));
    }

    #[test]
    fn parses_hex_colors() {
        assert_eq!(Color::parse_hex("#1a1b26"), Some(Color::rgb(0x1a1b26)));
        assert_eq!(Color::parse_hex("#f0a"), Some(Color::rgb(0xff00aa)));
        assert_eq!(Color::rgba(0xffffff40).to_hex(), "#ffffff40");
        assert_eq!(Color::rgb(0x1a1b26).to_hex(), "#1a1b26");
        assert_eq!(Color::parse_hex("#ffffff40"), Some(Color::rgba(0xffffff40)));
        assert_eq!(Color::parse_hex("1a1b26"), None);
        assert_eq!(Color::parse_hex("#12345"), None);
        assert_eq!(Color::parse_hex("#gggggg"), None);
    }

    #[test]
    fn mixing_interpolates_channels() {
        let black = Color::rgb(0);
        let white = Color::rgb(0xffffff);
        assert_eq!(black.mix(white, 0.0), black);
        assert_eq!(black.mix(white, 1.0), white);
        assert_eq!(black.mix(white, 0.5), Color::rgb(0x808080));
    }

    #[test]
    fn light_and_dark_backgrounds_are_classified() {
        assert!(!scheme("Tokyo Night").background.is_light());
        assert!(scheme("One Half Light").background.is_light());
        assert!(scheme("Solarized Light").background.is_light());
        assert!(!scheme("Campbell").background.is_light());
    }

    #[test]
    fn the_default_scheme_matches_the_previous_implementation() {
        let tokyo = scheme("Tokyo Night");
        assert_eq!(tokyo.background, Color::rgb(0x1a1b26));
        assert_eq!(tokyo.ansi[4], Color::rgb(0x7aa2f7));
        assert_eq!(tokyo.ansi[15], Color::rgb(0xc0caf5));
    }

    #[test]
    fn campbell_selection_keeps_its_alpha() {
        assert_eq!(scheme("Campbell").selection.a, 0x40);
    }
}
