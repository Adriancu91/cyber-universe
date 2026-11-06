//! A tolerant HTML-to-text renderer (P6-53, C-68 F6a), written for Cyber Universe from the public WHATWG HTML
//! specification (only as a reference for tag and entity names; no code from other browsers). It understands
//! enough for simple pages and documentation: the title, headings, paragraphs, lists, quotes, preformatted text,
//! simple tables, images (their `alt` text), links, which are numbered `[N]` in the text, and (F6b, P6-61) forms:
//! `[formular F1]`, fields `[F1.nume: valoare]`, buttons `[F1: buton "..."]`. Scripts, styles and
//! the rest of `<head>` are skipped; broken markup never stops it (it just becomes text or is ignored).
//! Included with `#[path]` by /bin/web; it only needs `alloc`.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// A page turned into text lines no wider than the width asked for.
pub struct Doc {
    pub title: String,
    pub lines: Vec<String>,
    /// The `href` of every numbered link, in order (link N is `links[N - 1]`), as written in the page.
    pub links: Vec<String>,
    /// F6b (P6-61 step 6): the page's forms, numbered `F1`, `F2`... in the text.
    pub forms: Vec<Form>,
}

/// A form: where it goes, how (`get` or `post`), and its fields with the values they have at the start
/// (hidden ones included; a checkbox or radio only when checked; a select with its selected or first option).
#[derive(Clone)]
pub struct Form {
    pub action: String,
    pub method: String,
    pub fields: Vec<(String, String)>,
}

/// Elements whose content is never shown.
const HIDDEN: [&str; 7] = ["script", "style", "head", "template", "svg", "iframe", "object"];
/// Elements that start and end a block of text.
const BLOCKS: [&str; 33] = [
    "p", "div", "section", "article", "header", "footer", "nav", "main", "aside", "h1", "h2", "h3", "h4", "h5", "h6",
    "ul", "ol", "li", "table", "tr", "blockquote", "pre", "hr", "form", "dl", "dt", "dd", "figure", "figcaption",
    "address", "body", "html", "details",
];

struct Builder {
    width: usize,
    lines: Vec<String>,
    /// The text of the block being built (spaces already collapsed, except in `<pre>`).
    text: String,
    links: Vec<String>,
    /// The `href` of the open `<a>`, if it gets a number.
    open_link: Option<String>,
    /// Open lists: true = numbered (`<ol>`), with its counter.
    lists: Vec<(bool, u32)>,
    /// The prefix of the first line of the current block (list bullet), the following lines get spaces.
    bullet: String,
    heading: u8,
    quote: usize,
    pre: usize,
    hidden: usize,
    /// Cells already written in the current table row.
    cells: usize,
    title: String,
    in_title: bool,
    forms: Vec<Form>,
    /// The open `<form>` (index in `forms`).
    form: Option<usize>,
    /// The open `<select>`: its field index and whether an option was marked `selected`.
    select: Option<(usize, bool)>,
    /// Inside a `<select>` (also one without a name): its options' text is not shown.
    in_select: bool,
}

impl Builder {
    /// The current form, or a form of its own for a field outside any `<form>` (it still shows, with a number).
    fn form_index(&mut self) -> usize {
        match self.form {
            Some(i) => i,
            None => {
                self.forms.push(Form { action: String::new(), method: String::from("get"), fields: Vec::new() });
                self.forms.len() - 1
            }
        }
    }

    fn field_label(&mut self, f: usize, name: &str, shown: &str) {
        let mut s = String::from(" [F");
        s.push_str(&(f + 1).to_string());
        if !name.is_empty() {
            s.push('.');
            s.push_str(name);
        }
        s.push_str(": ");
        s.push_str(shown);
        s.push_str("] ");
        self.push_text(&s);
    }

    /// `<input>`, `<button>`, `<textarea>` (with its text), `<select>` / `<option>`.
    fn form_tag(&mut self, name: &str, attrs: &str, text: Option<&str>) {
        let field = attr(attrs, "name").map(decode).unwrap_or_default();
        let value = attr(attrs, "value").map(decode).unwrap_or_default();
        match name {
            "input" | "button" => {
                let kind = attr(attrs, "type").map(|t| t.to_ascii_lowercase()).unwrap_or_else(|| {
                    String::from(if name == "button" { "submit" } else { "text" })
                });
                let f = self.form_index();
                match kind.as_str() {
                    "hidden" => {
                        if !field.is_empty() {
                            self.forms[f].fields.push((field, value));
                        }
                    }
                    "submit" | "button" | "image" | "reset" => {
                        let inner = text.map(|t| decode(t).split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
                        let label = if !value.is_empty() { value } else if !inner.is_empty() { inner } else { String::from("Trimite") };
                        let mut s = String::from("buton \"");
                        s.push_str(label.trim());
                        s.push('"');
                        self.field_label(f, "", &s);
                    }
                    "checkbox" | "radio" => {
                        let on = has_attr(attrs, "checked");
                        let v = if value.is_empty() { String::from("on") } else { value };
                        let mut s = String::from(if on { "(x) " } else { "( ) " });
                        s.push_str(&v);
                        self.field_label(f, &field, &s);
                        if on && !field.is_empty() {
                            self.forms[f].fields.push((field, v));
                        }
                    }
                    _ => {
                        let shown = if kind == "password" { "*".repeat(value.chars().count()) } else { value.clone() };
                        let shown = if shown.is_empty() { String::from("___") } else { shown };
                        self.field_label(f, &field, &shown);
                        if !field.is_empty() {
                            self.forms[f].fields.push((field, value));
                        }
                    }
                }
            }
            "textarea" => {
                let f = self.form_index();
                let v = decode(text.unwrap_or("")).trim().to_string();
                let shown = if v.is_empty() { String::from("___") } else { v.clone() };
                self.field_label(f, &field, &shown);
                if !field.is_empty() {
                    self.forms[f].fields.push((field, v));
                }
            }
            "select" => {
                let f = self.form_index();
                self.field_label(f, &field, "alegere");
                self.in_select = true;
                if field.is_empty() {
                    self.select = None;
                } else {
                    self.forms[f].fields.push((field, String::new()));
                    self.select = Some((self.forms[f].fields.len() - 1, false));
                }
            }
            "option" => {
                if let (Some((i, chosen)), Some(f)) = (self.select, self.form.or(self.forms.len().checked_sub(1))) {
                    let selected = has_attr(attrs, "selected");
                    let first = self.forms[f].fields[i].1.is_empty() && !chosen;
                    if selected || first {
                        self.forms[f].fields[i].1 = value;
                        self.select = Some((i, chosen || selected));
                    }
                }
            }
            _ => {}
        }
    }

    fn indent(&self) -> String {
        let mut s = String::new();
        for _ in 0..self.quote {
            s.push_str("> ");
        }
        for _ in 0..self.lists.len().saturating_sub(1) {
            s.push_str("  ");
        }
        s
    }

    fn blank(&mut self) {
        if self.lines.last().is_some_and(|l| !l.is_empty()) {
            self.lines.push(String::new());
        }
    }

    /// Ends the current block: its text becomes wrapped lines.
    fn flush(&mut self) {
        let text = core::mem::take(&mut self.text);
        let bullet = core::mem::take(&mut self.bullet);
        if self.pre > 0 {
            let ind = self.indent();
            for l in text.trim_matches('\n').split('\n') {
                let mut line = ind.clone();
                line.push_str(l.trim_end());
                self.lines.push(line);
            }
            return;
        }
        let words: Vec<&str> = text.split(' ').filter(|w| !w.is_empty()).collect();
        if words.is_empty() {
            return;
        }
        let ind = self.indent();
        let mut first = ind.clone();
        let mut rest = ind;
        if self.heading > 0 {
            for _ in 0..self.heading {
                first.push('#');
            }
            first.push(' ');
            self.blank();
        } else if !bullet.is_empty() {
            first.push_str(&bullet);
            for _ in 0..chars(&bullet) {
                rest.push(' ');
            }
        }
        let mut line = first;
        let mut empty = true;
        for w in words {
            if !empty && chars(&line) + 1 + chars(w) > self.width {
                self.lines.push(core::mem::replace(&mut line, rest.clone()));
                empty = true;
            }
            if !empty {
                line.push(' ');
            }
            line.push_str(w);
            empty = false;
        }
        self.lines.push(line);
        if self.heading > 0 {
            self.blank();
        }
    }

    fn push_text(&mut self, s: &str) {
        if self.select.is_some() || self.in_select {
            return; // the options of a <select> show as one field, not as text
        }
        if self.in_title {
            for c in s.chars() {
                if c.is_whitespace() {
                    if !self.title.ends_with(' ') && !self.title.is_empty() {
                        self.title.push(' ');
                    }
                } else {
                    self.title.push(c);
                }
            }
            return;
        }
        if self.hidden > 0 {
            return;
        }
        if self.pre > 0 {
            self.text.push_str(s);
            return;
        }
        for c in s.chars() {
            if c.is_whitespace() && c != '\u{a0}' {
                if !self.text.is_empty() && !self.text.ends_with(' ') {
                    self.text.push(' ');
                }
            } else {
                self.text.push(if c == '\u{a0}' { ' ' } else { c });
            }
        }
    }

    fn start_block(&mut self, paragraph: bool) {
        self.flush();
        if paragraph && self.lists.is_empty() {
            self.blank();
        }
    }

    fn tag(&mut self, name: &str, end: bool, attrs: &str) {
        if name == "body" && !end {
            // Pages that never close <head> (allowed by HTML) still show their body.
            self.hidden = 0;
        }
        if HIDDEN.contains(&name) {
            if end {
                self.hidden = self.hidden.saturating_sub(1);
            } else {
                self.hidden += 1;
            }
            return;
        }
        if name == "title" {
            // The title lives in <head>, which is hidden: it is read on its own.
            self.in_title = !end && self.title.is_empty();
            return;
        }
        if self.hidden > 0 {
            return;
        }
        match (name, end) {
            ("br", _) => {
                if self.pre > 0 {
                    self.text.push('\n');
                } else {
                    self.flush();
                }
            }
            ("a", false) => {
                let href = attr(attrs, "href").map(|h| decode(h.trim()));
                self.open_link = href.filter(|h| {
                    let l = h.to_ascii_lowercase();
                    !h.is_empty() && !h.starts_with('#') && !l.starts_with("javascript:") && !l.starts_with("mailto:")
                });
            }
            ("a", true) => {
                if let Some(h) = self.open_link.take() {
                    self.links.push(h);
                    let n = self.links.len();
                    let mut s = String::from("[");
                    s.push_str(&n.to_string());
                    s.push(']');
                    if !self.text.ends_with(' ') && self.pre == 0 && !self.text.is_empty() {
                        self.text.push(' ');
                    }
                    self.text.push_str(&s);
                }
            }
            ("img", _) => {
                if let Some(alt) = attr(attrs, "alt").map(decode).filter(|a| !a.trim().is_empty()) {
                    self.push_text(" [imagine: ");
                    self.push_text(alt.trim());
                    self.push_text("] ");
                }
            }
            ("ul" | "ol", false) => {
                self.start_block(true);
                self.lists.push((name == "ol", 0));
            }
            ("ul" | "ol", true) => {
                self.flush();
                self.lists.pop();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            ("li", false) => {
                self.flush();
                let bullet = match self.lists.last_mut() {
                    Some((true, n)) => {
                        *n += 1;
                        let mut b = n.to_string();
                        b.push_str(". ");
                        b
                    }
                    _ => "* ".to_string(),
                };
                self.bullet = bullet;
            }
            ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", false) => {
                self.start_block(true);
                self.heading = name.as_bytes()[1] - b'0';
            }
            ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", true) => {
                self.flush();
                self.heading = 0;
            }
            ("blockquote", false) => {
                self.start_block(true);
                self.quote += 1;
            }
            ("blockquote", true) => {
                self.flush();
                self.quote = self.quote.saturating_sub(1);
                self.blank();
            }
            ("pre", false) => {
                self.start_block(true);
                self.pre += 1;
            }
            ("pre", true) => {
                self.flush();
                self.pre = self.pre.saturating_sub(1);
                self.blank();
            }
            ("hr", _) => {
                self.flush();
                let mut l = self.indent();
                for _ in 0..self.width.min(40) {
                    l.push('-');
                }
                self.lines.push(l);
            }
            ("tr", _) => {
                self.flush();
                self.cells = 0;
            }
            ("td" | "th", false) => {
                if self.cells > 0 {
                    self.text.push_str(" | ");
                }
                self.cells += 1;
            }
            ("form", false) => {
                self.start_block(true);
                let action = attr(attrs, "action").map(|a| decode(a.trim())).unwrap_or_default();
                let method = attr(attrs, "method").map(|m| m.trim().to_ascii_lowercase()).filter(|m| m == "post").unwrap_or_else(|| String::from("get"));
                self.forms.push(Form { action, method, fields: Vec::new() });
                self.form = Some(self.forms.len() - 1);
                let mut s = String::from("[formular F");
                s.push_str(&self.forms.len().to_string());
                s.push(']');
                self.push_text(&s);
                self.flush();
            }
            ("form", true) => {
                self.start_block(true);
                self.form = None;
                self.select = None;
            }
            ("input" | "select" | "option", false) => self.form_tag(name, attrs, None),
            ("select", true) => {
                self.select = None;
                self.in_select = false;
            }
            ("p" | "div" | "table" | "dl" | "figure" | "section" | "article" | "address", _) => {
                self.start_block(true);
            }
            ("dd", false) => {
                self.flush();
                self.bullet = "    ".to_string();
            }
            _ if BLOCKS.contains(&name) => self.flush(),
            _ => {}
        }
    }
}

/// Whether a boolean attribute (`checked`, `selected`) is present.
fn has_attr(attrs: &str, name: &str) -> bool {
    let low = attrs.to_ascii_lowercase();
    low.split(|c: char| c.is_ascii_whitespace() || c == '/' || c == '"' || c == '\'')
        .any(|w| w == name || w.strip_prefix(name).is_some_and(|r| r.starts_with('=')))
}

/// `application/x-www-form-urlencoded` (WHATWG URL standard): letters, digits and `*-._` stay, space is `+`.
pub fn form_encode(fields: &[(String, String)]) -> String {
    let mut out = String::new();
    for (i, (k, v)) in fields.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        for (j, part) in [k, v].iter().enumerate() {
            if j == 1 {
                out.push('=');
            }
            for b in part.bytes() {
                match b {
                    b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => out.push(b as char),
                    b' ' => out.push('+'),
                    _ => {
                        const H: &[u8; 16] = b"0123456789ABCDEF";
                        out.push('%');
                        out.push(H[(b >> 4) as usize] as char);
                        out.push(H[(b & 15) as usize] as char);
                    }
                }
            }
        }
    }
    out
}

fn chars(s: &str) -> usize {
    s.chars().count()
}

/// The value of attribute `name` in the inside of a tag (`href="x" class=y`), quoted or not.
fn attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let b = attrs.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'>' {
            i += 1;
        }
        let key = &attrs[start..i];
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = "";
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                i += 1;
                let vs = i;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                value = &attrs[vs..i];
                i += 1;
            } else {
                let vs = i;
                while i < b.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                value = &attrs[vs..i];
            }
        }
        if key.eq_ignore_ascii_case(name) {
            return Some(value);
        }
        if i == start {
            i += 1;
        }
    }
    None
}

/// The named character references most pages use (WHATWG list, a small part of it), Romanian letters included.
const ENTITIES: [(&str, char); 40] = [
    ("amp", '&'), ("lt", '<'), ("gt", '>'), ("quot", '"'), ("apos", '\''), ("nbsp", '\u{a0}'), ("copy", '©'),
    ("reg", '®'), ("trade", '™'), ("hellip", '…'), ("mdash", '—'), ("ndash", '–'), ("laquo", '«'), ("raquo", '»'),
    ("lsquo", '‘'), ("rsquo", '’'), ("ldquo", '“'), ("rdquo", '”'), ("bdquo", '„'), ("bull", '•'), ("middot", '·'),
    ("euro", '€'), ("deg", '°'), ("times", '×'), ("larr", '←'), ("rarr", '→'), ("para", '¶'), ("sect", '§'),
    ("Acirc", 'Â'), ("acirc", 'â'), ("Icirc", 'Î'), ("icirc", 'î'), ("Abreve", 'Ă'), ("abreve", 'ă'),
    ("Scedil", 'Ş'), ("scedil", 'ş'), ("Tcedil", 'Ţ'), ("tcedil", 'ţ'), ("shy", '\u{ad}'), ("ensp", ' '),
];

/// Replaces character references (`&amp;`, `&#259;`, `&#x219;`); unknown ones stay as written.
pub fn decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest[1..].find(|c: char| c == ';' || c == '&' || c.is_whitespace() || c == '<').map(|e| e + 1);
        let semi = end.filter(|&e| rest.as_bytes()[e] == b';');
        let Some(e) = semi.filter(|&e| e <= 12) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..e];
        let c = if let Some(num) = name.strip_prefix('#') {
            let v = match num.strip_prefix(['x', 'X']) {
                Some(h) => u32::from_str_radix(h, 16).ok(),
                None => num.parse::<u32>().ok(),
            };
            v.map(|v| char::from_u32(v).filter(|&c| c != '\0').unwrap_or('\u{fffd}'))
        } else {
            ENTITIES.iter().find(|(n, _)| *n == name).map(|&(_, c)| c)
        };
        match c {
            Some('\u{ad}') => {}
            Some(c) => out.push(c),
            None => out.push_str(&rest[..=e]),
        }
        rest = &rest[e + 1..];
    }
    out.push_str(rest);
    out
}

/// Turns an HTML page into text lines at most `width` characters wide (longer words stay whole).
pub fn render(html: &str, width: usize) -> Doc {
    let mut b = Builder {
        width: width.max(20),
        lines: Vec::new(),
        text: String::new(),
        links: Vec::new(),
        open_link: None,
        lists: Vec::new(),
        bullet: String::new(),
        heading: 0,
        quote: 0,
        pre: 0,
        hidden: 0,
        cells: 0,
        title: String::new(),
        in_title: false,
        forms: Vec::new(),
        form: None,
        select: None,
        in_select: false,
    };
    let mut rest = html;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            b.push_text(&decode(rest));
            break;
        };
        if lt > 0 {
            b.push_text(&decode(&rest[..lt]));
        }
        rest = &rest[lt..];
        if let Some(c) = rest.strip_prefix("<!--") {
            rest = c.find("-->").map_or("", |e| &c[e + 3..]);
            continue;
        }
        let next = rest.as_bytes().get(1).copied().unwrap_or(0);
        if next == b'!' || next == b'?' {
            rest = rest.find('>').map_or("", |e| &rest[e + 1..]);
            continue;
        }
        let end = next == b'/';
        let start = if end { 2 } else { 1 };
        if !rest.as_bytes().get(start).is_some_and(|c| c.is_ascii_alphabetic()) {
            // A lone "<" (as in "a < b"): text.
            b.push_text("<");
            rest = &rest[1..];
            continue;
        }
        let close = tag_end(rest).unwrap_or(rest.len());
        let inner = &rest[start..close.min(rest.len())];
        let name_len = inner.find(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>').unwrap_or(inner.len());
        let name = inner[..name_len].to_ascii_lowercase();
        let attrs = &inner[name_len..];
        rest = if close < rest.len() { &rest[close + 1..] } else { "" };
        // Raw text elements: their content is not markup, jump to their end tag.
        if !end && (name == "script" || name == "style") {
            let pat = if name == "script" { "</script" } else { "</style" };
            let pos = find_ci(rest, pat);
            rest = pos.map_or("", |p| &rest[p..]);
            continue;
        }
        if !end && name == "button" {
            // The button's text is its label (tags inside it dropped).
            let pos = find_ci(rest, "</button").unwrap_or(rest.len());
            let mut inner = String::new();
            let mut in_tag = false;
            for c in rest[..pos].chars() {
                match c {
                    '<' => in_tag = true,
                    '>' => in_tag = false,
                    _ if !in_tag => inner.push(c),
                    _ => {}
                }
            }
            b.form_tag("button", attrs, Some(&inner));
            rest = &rest[pos..];
            rest = rest.find('>').map_or("", |e| &rest[e + 1..]);
            continue;
        }
        if !end && name == "textarea" {
            let pos = find_ci(rest, "</textarea").unwrap_or(rest.len());
            b.form_tag("textarea", attrs, Some(&rest[..pos]));
            rest = &rest[pos..];
            rest = rest.find('>').map_or("", |e| &rest[e + 1..]);
            continue;
        }
        if !end && name == "title" {
            let pos = find_ci(rest, "</title").unwrap_or(rest.len());
            b.tag("title", false, "");
            b.push_text(&decode(&rest[..pos]));
            b.tag("title", true, "");
            rest = &rest[pos..];
            rest = rest.find('>').map_or("", |e| &rest[e + 1..]);
            continue;
        }
        b.tag(&name, end, attrs);
    }
    b.flush();
    while b.lines.last().is_some_and(|l| l.is_empty()) {
        b.lines.pop();
    }
    while b.lines.first().is_some_and(|l| l.is_empty()) {
        b.lines.remove(0);
    }
    Doc { title: b.title.trim().to_string(), lines: b.lines, links: b.links, forms: b.forms }
}

/// The position of the `>` that ends the tag starting at `s[0]`, skipping `>` inside quoted attribute values.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote = 0u8;
    for (i, &c) in s.as_bytes().iter().enumerate().skip(1) {
        match (quote, c) {
            (0, b'"' | b'\'') => quote = c,
            (0, b'>') => return Some(i),
            (q, c) if q == c => quote = 0,
            _ => {}
        }
    }
    None
}

/// Case-insensitive search for an ASCII pattern.
fn find_ci(s: &str, pat: &str) -> Option<usize> {
    let (h, p) = (s.as_bytes(), pat.as_bytes());
    (0..h.len().saturating_sub(p.len() - 1)).find(|&i| h[i..i + p.len()].eq_ignore_ascii_case(p))
}

/// The address `href` points to, seen from the page at `base` ("http(s)://host[:port]/path"): absolute addresses
/// stay, "//host/x" takes the scheme, "/x" the host, "?q" the path, the rest is relative to the page's folder
/// ("./" and "../" removed). The "#part" is dropped. None for schemes other than http and https.
pub fn resolve(base: &str, href: &str) -> Option<String> {
    let href = href.split('#').next().unwrap_or("").trim();
    let lower = href.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(href.to_string());
    }
    if let Some(colon) = href.find(':')
        && href[..colon].bytes().all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'-' || c == b'.')
        && !href[..colon].contains('/')
        && colon > 0
    {
        return None; // mailto:, ftp:, data: ...
    }
    let scheme_end = base.find("://")? + 3;
    let path_start = base[scheme_end..].find('/').map_or(base.len(), |p| p + scheme_end);
    let origin = &base[..path_start];
    let base_path = base[path_start..].split('#').next().unwrap_or("");
    let base_path = if base_path.is_empty() { "/" } else { base_path };
    if let Some(rest) = href.strip_prefix("//") {
        let mut s = base[..scheme_end].to_string();
        s.push_str(rest);
        return Some(s);
    }
    let without_query = base_path.split('?').next().unwrap_or("/");
    let joined = if href.is_empty() {
        base_path.to_string()
    } else if href.starts_with('?') {
        let mut s = without_query.to_string();
        s.push_str(href);
        s
    } else if href.starts_with('/') {
        href.to_string()
    } else {
        let dir = &without_query[..without_query.rfind('/').map_or(0, |i| i + 1)];
        let mut s = if dir.is_empty() { "/".to_string() } else { dir.to_string() };
        s.push_str(href);
        s
    };
    let (path, query) = match joined.find('?') {
        Some(q) => (&joined[..q], &joined[q..]),
        None => (joined.as_str(), ""),
    };
    let mut parts: Vec<&str> = Vec::new();
    let segs: Vec<&str> = path.split('/').skip(1).collect();
    for (i, seg) in segs.iter().enumerate() {
        let last = i + 1 == segs.len();
        match *seg {
            "." => {
                if last {
                    parts.push("");
                }
            }
            ".." => {
                parts.pop();
                if last {
                    parts.push("");
                }
            }
            s => parts.push(s),
        }
    }
    let mut out = origin.to_string();
    for p in &parts {
        out.push('/');
        out.push_str(p);
    }
    if parts.is_empty() {
        out.push('/');
    }
    out.push_str(query);
    Some(out)
}
