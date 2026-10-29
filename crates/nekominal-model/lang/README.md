# Languages of the interface

Each file here holds the words of one language: an object from the id of a string to its text, named by the
language's tag (`en-US.json`, `zh-TW.json`). The files are built into the program; nothing is read from disk at run
time.

The ids, and a comment on what each string is for, are in `src/strings.rs`. A string a language does not have shows
in English, so a translation can be partial. English and Traditional Chinese must have every string: the tests say
which one is missing.

## Changing a string

Change its text in every file that has it. A new string is a row in `src/strings.rs` (its variant and id, under a
comment that says where it shows) and a line in `en-US.json` and `zh-TW.json`.

Keep the `{0}`, `{1}` of the English text: the program puts the numbers and names there. Labels end with no full
stop, and only the questions of a confirmation (`dialog.*`) end with a question mark; the tests check both.

## Adding a language

Japanese (`ja-JP`) as the example:

1. Copy `en-US.json` to `ja-JP.json` and translate the texts. Leave the ids as they are.
2. In `src/strings.rs`, add `JaJp` to `Lang`, to `Lang::ALL`, to `Lang::tag` (`"ja-JP"`), to the function `file`
   (`include_str!("../lang/ja-JP.json")`), and to `Lang::from_system_tag` so that a Japanese Windows picks it.
3. Add the row `LanguageJaJp "language.jaJp";` next to `LanguageZhTw`, with the language's own name (`日本語`) as its
   text in every file: a language is listed in its own words whatever the language of the interface.
4. In `src/settings.rs`, add `JaJp` to `Language` (with `#[serde(rename = "ja-JP")]`), to `Language::ALL`,
   `resolve` and `text`, and `("ja-JP", Language::JaJp)` to the list `language` is read with. In
   `src/settings_ui.rs`, add `Text::LanguageJaJp` to `LANGUAGES`.
5. Run `cargo test -p nekominal-model`. The test of the language names lists each language with its own name: add
   the new one there.

The release notes the About page shows come from GitHub in English and Traditional Chinese; another language shows
the English ones. The website has its own words, in `site/i18n.js`.
