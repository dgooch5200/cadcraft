//! Self-contained HTML pages for the BOM and patch exports: inline CSS, no external resources.

use serde_json::Value;

use crate::Session;

pub(super) fn escape_html(t: &str) -> String {
    t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// The footer lines printed under a table: the request's `footer`, else the title block's project fields.
pub(super) fn footer_lines(s: &Session, p: &Value) -> Vec<String> {
    match p.get("footer").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_str).map(String::from).collect(),
        None => super::sheet::project_footer(s),
    }
}

/// One table row; every cell is escaped. `class` is a fixed CSS class, never user text.
pub(super) fn row(class: &str, cells: &[String]) -> String {
    let tds: String = cells.iter().map(|c| format!("<td>{}</td>", escape_html(c))).collect();
    format!("<tr class=\"{class}\">{tds}</tr>\n")
}

/// A full-width group header row (a universe, say) with a note.
pub(super) fn group_row(class: &str, cols: usize, title: &str, note: &str) -> String {
    format!("<tr class=\"{class}\"><td colspan=\"{cols}\">{} <span class=\"note\">{}</span></td></tr>\n", escape_html(title), escape_html(note))
}

/// The page: title, an info box of facts, the table (`body` is finished rows) and the footer lines.
pub(super) fn page(title: &str, info: &[(&str, String)], heads: &[&str], body: &str, footer: &[String]) -> String {
    let info: String = info.iter().map(|(k, v)| format!("<tr><th>{}</th><td>{}</td></tr>", escape_html(k), escape_html(v))).collect();
    let heads: String = heads.iter().map(|h| format!("<th>{}</th>", escape_html(h))).collect();
    let foot: String = footer.iter().map(|l| format!("<div>{}</div>", escape_html(l))).collect();
    let title = escape_html(title);
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>{title}</title>\n<style>\n\
@page {{ size: letter landscape; margin: 0.5in; }}\n\
body {{ font: 10pt Helvetica, Arial, sans-serif; color: #111; margin: 0; }}\n\
h1 {{ font-size: 16pt; margin: 0 0 10px; }}\n\
.info {{ border: 1px solid #ccc; padding: 6px 10px; margin-bottom: 12px; }}\n\
.info th {{ text-align: left; padding-right: 12px; }}\n\
table.main {{ width: 100%; border-collapse: collapse; }}\n\
table.main th {{ text-align: left; border-bottom: 2px solid #333; padding: 4px; }}\n\
table.main td {{ border-bottom: 1px solid #ddd; padding: 3px 4px; vertical-align: top; }}\n\
tr.group td {{ background: #eee; font-weight: 600; border-top: 1px solid #999; }}\n\
tr.cont td {{ color: #666; font-style: italic; }}\n\
.note {{ font-weight: 400; color: #555; margin-left: 12px; }}\n\
.foot {{ margin-top: 14px; border-top: 1px solid #999; padding-top: 6px; font-size: 9pt; color: #333; }}\n\
tr {{ break-inside: avoid; }}\n\
</style></head><body>\n<h1>{title}</h1>\n<table class=\"info\">{info}</table>\n\
<table class=\"main\"><thead><tr>{heads}</tr></thead>\n<tbody>\n{body}</tbody></table>\n<div class=\"foot\">{foot}</div>\n</body></html>\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_markup_and_ampersands() {
        assert_eq!(escape_html("<script>alert('x')</script> & \"q\""), "&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt; &amp; &quot;q&quot;");
    }
}
