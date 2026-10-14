//! The interface language and the bridge from `nekominal_model::strings` to XAML markup.
//!
//! Every string lives in the model (one row per string, both languages), so a label that names an action is
//! the same text in the tab strip, the tool cluster, the palette and the menus. XAML templates carry
//! `{{id}}` placeholders (the string ids of the model) that [`localize`] replaces before `XamlReader::Load`.

use std::cell::Cell;
use std::fmt::Display;

use nekominal_model::strings::{Lang, Text};

use crate::util::xml_escape;

thread_local! {
    static LANG: Cell<Lang> = const { Cell::new(Lang::ZhTw) };
}

pub fn set_lang(lang: Lang) {
    LANG.with(|l| l.set(lang));
}

pub fn lang() -> Lang {
    LANG.with(Cell::get)
}

pub fn t(text: Text) -> &'static str {
    text.get(lang())
}

pub fn tf(text: Text, args: &[&dyn Display]) -> String {
    text.format(lang(), args)
}

/// Replaces every `{{id}}` in a XAML template with the string of that id, XML-escaped. An unknown id stays
/// visible as `?id?` so a typo shows up in the interface.
pub fn localize(markup: &str) -> String {
    let mut out = String::with_capacity(markup.len());
    let mut rest = markup;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let id = &after[..end];
        match Text::from_id(id) {
            Some(text) => out.push_str(&xml_escape(t(text))),
            None => out.push_str(&format!("?{id}?")),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_replaced_by_the_string_of_that_id() {
        set_lang(Lang::EnUs);
        assert_eq!(localize(r#"<A Name="{{tab.new}}"/>"#), r#"<A Name="New tab"/>"#);
        set_lang(Lang::ZhTw);
        assert_eq!(localize("{{tab.new}}|{{edit.find}}"), "新增分頁|尋找");
    }

    #[test]
    fn unknown_ids_stay_visible_and_text_is_escaped() {
        set_lang(Lang::EnUs);
        assert_eq!(localize("{{nope}}"), "?nope?");
        assert_eq!(localize("a {{ unterminated"), "a {{ unterminated");
        set_lang(Lang::ZhTw);
    }
}
