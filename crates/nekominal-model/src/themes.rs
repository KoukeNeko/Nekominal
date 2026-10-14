//! Terminal color schemes.

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

    /// Parses `#rrggbb` or `#rrggbbaa`.
    pub fn parse_hex(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('#')?;
        if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        match digits.len() {
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
}

#[derive(Clone, Copy, Debug)]
pub struct Scheme {
    pub name: &'static str,
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
        name: "Tokyo Night",
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
        name: "Campbell",
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
        name: "One Half Dark",
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
        name: "Catppuccin Mocha",
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
        name: "Dracula",
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
        name: "Nord",
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
        name: "Gruvbox Dark",
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
        name: "Solarized Dark",
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
        name: "One Half Light",
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
        name: "Solarized Light",
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

pub fn theme_names() -> impl Iterator<Item = &'static str> {
    SCHEMES.iter().map(|scheme| scheme.name)
}

/// The scheme called `name`, or the default one when the name is unknown.
pub fn scheme(name: &str) -> &'static Scheme {
    SCHEMES
        .iter()
        .find(|scheme| scheme.name == name)
        .unwrap_or_else(|| SCHEMES.iter().find(|s| s.name == DEFAULT_THEME).expect("default theme exists"))
}

pub fn is_known(name: &str) -> bool {
    SCHEMES.iter().any(|scheme| scheme.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_names_are_unique_and_include_the_default() {
        let names: Vec<_> = theme_names().collect();
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
        assert!(is_known("Dracula"));
        assert!(!is_known("nope"));
    }

    #[test]
    fn parses_hex_colors() {
        assert_eq!(Color::parse_hex("#1a1b26"), Some(Color::rgb(0x1a1b26)));
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
