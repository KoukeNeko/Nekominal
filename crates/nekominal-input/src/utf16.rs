//! `CharacterReceived` (WinUI), `WM_CHAR` and TSF deliver UTF-16 code units: a character outside the BMP
//! (emoji, CJK extension B) arrives as two events, high surrogate first. The adapter feeds every unit here.

/// Pairs UTF-16 surrogates into `char`s. An unpaired surrogate is dropped (U+FFFD would corrupt the input line).
#[derive(Clone, Copy, Debug, Default)]
pub struct Utf16Assembler {
    high: Option<u16>,
}

impl Utf16Assembler {
    pub fn push(&mut self, unit: u16) -> Option<char> {
        match unit {
            0xD800..=0xDBFF => {
                self.high = Some(unit);
                None
            }
            0xDC00..=0xDFFF => {
                let high = self.high.take()?;
                char::decode_utf16([high, unit]).next().and_then(Result::ok)
            }
            _ => {
                self.high = None;
                char::from_u32(u32::from(unit))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(units: &[u16]) -> String {
        let mut assembler = Utf16Assembler::default();
        units.iter().filter_map(|&u| assembler.push(u)).collect()
    }

    #[test]
    fn bmp_characters_pass_through() {
        assert_eq!(feed(&['a' as u16, 0x4F60, 0x3105]), "a你ㄅ");
    }

    #[test]
    fn surrogate_pairs_become_one_character() {
        assert_eq!(feed(&[0xD83D, 0xDE00]), "😀");
        assert_eq!(feed(&[0xD840, 0xDC0B, b'x' as u16]), "\u{2000B}x");
    }

    #[test]
    fn unpaired_surrogates_are_dropped() {
        assert_eq!(feed(&[0xDE00, b'a' as u16]), "a");
        assert_eq!(feed(&[0xD83D, b'a' as u16]), "a");
    }
}