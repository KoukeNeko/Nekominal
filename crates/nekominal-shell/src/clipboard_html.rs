//! The HTML of a copy with formatting, which the clipboard of either system takes along with the text.

/// The HTML of a copy with formatting: the lines of runs of the selection in a `<pre>` in the terminal's font and
/// colours, each run in its own colours and style. `resolve` turns the symbolic colours into RGB (foreground, background).
pub fn html_fragment(lines: &[Vec<nekominal_term::StyledRun>], font: &str, resolve: impl Fn(&nekominal_term::StyledRun) -> ((u8, u8, u8), (u8, u8, u8)), default: ((u8, u8, u8), (u8, u8, u8))) -> String {
    let hex = |(r, g, b): (u8, u8, u8)| format!("#{r:02x}{g:02x}{b:02x}");
    let escape = |text: &str| text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let mut html = format!(
        "<pre style=\"font-family:'{}',Consolas,monospace;color:{};background-color:{};padding:8px;white-space:pre\">",
        font.replace(['\'', '"', '<', '>'], ""),
        hex(default.0),
        hex(default.1)
    );
    for (n, runs) in lines.iter().enumerate() {
        if n > 0 {
            html.push('\n');
        }
        for run in runs {
            let (fg, bg) = resolve(run);
            let mut style = String::new();
            if fg != default.0 {
                style += &format!("color:{};", hex(fg));
            }
            if bg != default.1 {
                style += &format!("background-color:{};", hex(bg));
            }
            for (on, css) in [(run.bold, "font-weight:bold;"), (run.italic, "font-style:italic;"), (run.underline, "text-decoration:underline;")] {
                if on {
                    style += css;
                }
            }
            if style.is_empty() {
                html += &escape(&run.text);
            } else {
                html += &format!("<span style=\"{style}\">{}</span>", escape(&run.text));
            }
        }
    }
    html += "</pre>";
    html
}

#[cfg(test)]
mod tests {
    use super::html_fragment;
    use nekominal_term::{CellColor, StyledRun};

    #[test]
    fn a_formatted_copy_keeps_the_colours_and_style_of_each_run_and_escapes_the_text() {
        let run = |text: &str, fg, bold| StyledRun { text: text.into(), fg, bg: CellColor::Default, bold, italic: false, underline: false };
        let lines = vec![vec![run("ok ", CellColor::Default, false), run("<error>", CellColor::Indexed(1), true)], vec![]];
        let html = html_fragment(&lines, "Cascadia Mono", |r| (if r.fg == CellColor::Indexed(1) { (255, 0, 0) } else { (200, 200, 200) }, (20, 20, 20)), ((200, 200, 200), (20, 20, 20)));
        assert!(html.starts_with("<pre style=\"font-family:'Cascadia Mono',Consolas,monospace;color:#c8c8c8;background-color:#141414;"));
        assert!(html.contains("ok <span style=\"color:#ff0000;font-weight:bold;\">&lt;error&gt;</span>\n</pre>"), "{html}");
    }
}
