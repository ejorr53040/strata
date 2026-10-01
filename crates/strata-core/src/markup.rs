//! The markup index: HTML elements and CSS rules, kept apart from the code graph because
//! selectors match elements instead of names resolving to definitions.
//!
//! Selector matching is static: pseudo-classes that depend on state or position (`:hover`,
//! `:first-child`) are assumed to match, so a match means "can apply", never a guess that it
//! cannot. `:not(x)` with a single compound argument is honoured.

use crate::graph::Source;
use serde::Serialize;
use tree_sitter::{Node, Parser};

pub fn is_markup(path: &str) -> bool {
    is_html(path) || is_css(path)
}

fn is_html(path: &str) -> bool {
    matches!(path.rsplit_once('.').map(|(_, e)| e), Some("html" | "htm"))
}

fn is_css(path: &str) -> bool {
    path.ends_with(".css")
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Element {
    pub path: String,
    pub line: usize,
    pub tag: String,
    pub id: Option<String>,
    pub classes: Vec<String>,
    #[serde(skip)]
    attrs: Vec<(String, String)>,
    #[serde(skip)]
    parent: Option<usize>,
    #[serde(skip)]
    prev_sibling: Option<usize>,
}

/// One selector of a rule (`a, b { }` is two rules) and the properties it declares.
#[derive(Debug, Clone, Serialize)]
pub struct Rule {
    pub selector: String,
    pub specificity: [u32; 3],
    pub path: String,
    pub line: usize,
    /// Property names; `!important` declarations read `name !important`.
    pub properties: Vec<String>,
    #[serde(skip)]
    sheet: usize,
    /// Applies only in a state (`:hover`) or to a pseudo-element (`::before`), so it never
    /// decides the element's own resting style.
    #[serde(skip)]
    conditional: bool,
}

#[derive(Debug, Serialize)]
pub struct Winner {
    pub selector: String,
    pub path: String,
    pub line: usize,
}

#[derive(Debug, Serialize)]
pub struct Cascade<'a> {
    pub element: &'a Element,
    /// Matching rules from lowest to highest precedence by specificity, then source order.
    pub rules: Vec<&'a Rule>,
    /// For each property, the rule whose declaration applies (`!important` first).
    pub winners: std::collections::BTreeMap<String, Winner>,
}

#[derive(Debug, Default)]
pub struct Markup {
    pub elements: Vec<Element>,
    pub rules: Vec<Rule>,
    /// Stylesheet paths: a `.css` file, or the HTML file holding a `<style>` block.
    sheets: Vec<String>,
    /// For each HTML file, the stylesheets it applies, in document order.
    pages: std::collections::BTreeMap<String, Vec<usize>>,
    /// Per HTML file, `<link rel=stylesheet>` hrefs (resolved after every sheet is known)
    /// interleaved with inline `<style>` sheets.
    page_refs: Vec<(String, SheetRef)>,
}

/// Cascade precedence of a matching rule: specificity, then the page's sheet order, then
/// position in the index (source order within a sheet).
type Precedence = ([u32; 3], usize, usize);

#[derive(Debug)]
enum SheetRef {
    Href(String),
    Inline(usize),
}

pub fn build(sources: &[Source]) -> Markup {
    let mut m = Markup::default();
    for s in sources.iter().filter(|s| is_css(&s.path)) {
        let sheet = m.add_sheet(&s.path);
        m.parse_css(&s.text, &s.path, sheet, 0);
    }
    for s in sources.iter().filter(|s| is_html(&s.path)) {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_html::LANGUAGE.into())
            .unwrap();
        m.pages.entry(s.path.clone()).or_default();
        if let Some(tree) = parser.parse(&s.text, None) {
            let mut st = Html {
                src: s.text.as_bytes(),
                path: &s.path,
                out: &mut m,
            };
            st.children(tree.root_node(), None);
        }
    }
    m.link_pages();
    m
}

impl Markup {
    fn add_sheet(&mut self, path: &str) -> usize {
        self.sheets.push(path.to_string());
        self.sheets.len() - 1
    }

    /// Rules of a stylesheet; `line_offset` places a `<style>` block inside its HTML file.
    fn parse_css(&mut self, text: &str, path: &str, sheet: usize, line_offset: usize) {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_css::LANGUAGE.into())
            .unwrap();
        let Some(tree) = parser.parse(text, None) else {
            return;
        };
        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            if node.kind() != "rule_set" {
                let mut c = node.walk();
                let kids: Vec<Node> = node.named_children(&mut c).collect();
                stack.extend(kids.into_iter().rev());
                continue;
            }
            let src = text.as_bytes();
            let mut properties = Vec::new();
            if let Some(block) = node
                .named_children(&mut node.walk())
                .find(|n| n.kind() == "block")
            {
                let mut c = block.walk();
                for d in block
                    .named_children(&mut c)
                    .filter(|d| d.kind() == "declaration")
                {
                    let mut dc = d.walk();
                    let kids: Vec<Node> = d.named_children(&mut dc).collect();
                    let Some(name) = kids.iter().find(|k| k.kind() == "property_name") else {
                        continue;
                    };
                    let name = name.utf8_text(src).unwrap_or_default().to_string();
                    let important = kids.iter().any(|k| k.kind() == "important");
                    properties.push(if important {
                        format!("{name} !important")
                    } else {
                        name
                    });
                }
            }
            let Some(sels) = node
                .named_children(&mut node.walk())
                .find(|n| n.kind() == "selectors")
            else {
                continue;
            };
            let mut c = sels.walk();
            for sel in sels.named_children(&mut c) {
                let selector = sel
                    .utf8_text(src)
                    .unwrap_or_default()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                let Some(parsed) = parse(&selector) else {
                    continue;
                };
                self.rules.push(Rule {
                    specificity: parsed.specificity(),
                    selector,
                    path: path.to_string(),
                    line: sel.start_position().row + 1 + line_offset,
                    properties: properties.clone(),
                    conditional: parsed.compounds.iter().any(|c| c.conditional),
                    sheet,
                });
            }
        }
    }

    /// Resolve each page's `<link>` hrefs to indexed stylesheets.
    fn link_pages(&mut self) {
        for (page, r) in std::mem::take(&mut self.page_refs) {
            let ids: Vec<usize> = match r {
                SheetRef::Inline(id) => vec![id],
                SheetRef::Href(href) => self.resolve_href(&page, &href),
            };
            self.pages.entry(page).or_default().extend(ids);
        }
    }

    /// `base.css`, `../css/x.css`, `/static/x.css` or Jinja's
    /// `{{ url_for('static', filename='x.css') }}`: the path relative to the page when it is
    /// indexed, else every indexed stylesheet with that file name.
    fn resolve_href(&self, page: &str, href: &str) -> Vec<usize> {
        let href = match href.split_once("filename=") {
            Some((_, rest)) => rest
                .trim_start_matches(['\'', '"'])
                .split(['\'', '"'])
                .next()
                .unwrap_or_default(),
            None => href.split(['?', '#']).next().unwrap_or_default(),
        };
        if href.contains("://") || href.starts_with("//") {
            return Vec::new();
        }
        let dir = page.rsplit_once('/').map_or("", |(d, _)| d);
        let mut parts: Vec<&str> = if href.starts_with('/') {
            Vec::new()
        } else {
            dir.split('/').filter(|p| !p.is_empty()).collect()
        };
        for seg in href.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                s => parts.push(s),
            }
        }
        let rel = parts.join("/");
        let css = |i: &usize| is_css(&self.sheets[*i]);
        if let Some(i) = (0..self.sheets.len())
            .filter(css)
            .find(|&i| self.sheets[i] == rel)
        {
            return vec![i];
        }
        let file = href.rsplit('/').next().unwrap_or(href);
        (0..self.sheets.len())
            .filter(css)
            .filter(|&i| {
                let p = &self.sheets[i];
                p == file || p.ends_with(&format!("/{file}"))
            })
            .collect()
    }

    /// The stylesheets that apply to elements of `page`: the ones it links and embeds, or
    /// every indexed stylesheet when it names none (a fragment included elsewhere).
    fn sheets_of(&self, page: &str) -> Vec<usize> {
        match self.pages.get(page) {
            Some(s) if !s.is_empty() => s.clone(),
            _ => (0..self.sheets.len())
                .filter(|&i| is_css(&self.sheets[i]))
                .collect(),
        }
    }

    /// Every rule that can apply to the element(s) starting at `path:line`, in cascade order.
    pub fn matching_rules(&self, path: &str, line: usize) -> Vec<Cascade<'_>> {
        let mut out = Vec::new();
        for (i, e) in self.elements.iter().enumerate() {
            if e.path != path || e.line != line {
                continue;
            }
            let sheets = self.sheets_of(path);
            // (rule index, precedence key): specificity, then sheet order, then position.
            let mut hits: Vec<(usize, Precedence)> = self
                .rules
                .iter()
                .enumerate()
                .filter_map(|(ri, r)| {
                    let order = sheets.iter().position(|&s| s == r.sheet)?;
                    let sel = parse(&r.selector)?;
                    self.matches(&sel, i)
                        .then_some((ri, (r.specificity, order, ri)))
                })
                .collect();
            hits.sort_by_key(|h| h.1);
            let mut winners = std::collections::BTreeMap::new();
            let mut best: std::collections::BTreeMap<String, (bool, Precedence)> =
                std::collections::BTreeMap::new();
            for (ri, key) in &hits {
                let r = &self.rules[*ri];
                if r.conditional {
                    continue;
                }
                for p in &r.properties {
                    let (name, important) = match p.strip_suffix(" !important") {
                        Some(n) => (n.to_string(), true),
                        None => (p.clone(), false),
                    };
                    let rank = (important, *key);
                    if best.get(&name).is_none_or(|b| rank > *b) {
                        best.insert(name.clone(), rank);
                        winners.insert(
                            name,
                            Winner {
                                selector: r.selector.clone(),
                                path: r.path.clone(),
                                line: r.line,
                            },
                        );
                    }
                }
            }
            out.push(Cascade {
                element: e,
                rules: hits.iter().map(|(ri, _)| &self.rules[*ri]).collect(),
                winners,
            });
        }
        out
    }

    /// Rules whose selector matches no element on any page that loads their stylesheet.
    pub fn unused_selectors(&self) -> Vec<Winner> {
        let loaded: Vec<(&String, Vec<usize>)> =
            self.pages.keys().map(|p| (p, self.sheets_of(p))).collect();
        self.rules
            .iter()
            .filter(|r| {
                let Some(sel) = parse(&r.selector) else {
                    return false;
                };
                !self.elements.iter().enumerate().any(|(i, e)| {
                    loaded
                        .iter()
                        .any(|(p, sheets)| **p == e.path && sheets.contains(&r.sheet))
                        && self.matches(&sel, i)
                })
            })
            .map(|r| Winner {
                selector: r.selector.clone(),
                path: r.path.clone(),
                line: r.line,
            })
            .collect()
    }

    /// Elements `selector` (one selector or a comma-separated list) can apply to.
    pub fn matched_elements(&self, selector: &str) -> Vec<&Element> {
        let sels: Vec<Selector> = split_list(selector)
            .iter()
            .filter_map(|s| parse(s))
            .collect();
        (0..self.elements.len())
            .filter(|&i| sels.iter().any(|s| self.matches(s, i)))
            .map(|i| &self.elements[i])
            .collect()
    }

    fn matches(&self, sel: &Selector, el: usize) -> bool {
        self.match_from(sel, sel.compounds.len() - 1, el)
    }

    /// Does `sel.compounds[..=k]` match with compound `k` on element `el`? Right to left.
    fn match_from(&self, sel: &Selector, k: usize, el: usize) -> bool {
        if !self.compound(&sel.compounds[k], el) {
            return false;
        }
        if k == 0 {
            return true;
        }
        let e = &self.elements[el];
        match sel.combinators[k - 1] {
            Comb::Child => e.parent.is_some_and(|p| self.match_from(sel, k - 1, p)),
            Comb::Descendant => {
                let mut p = e.parent;
                while let Some(i) = p {
                    if self.match_from(sel, k - 1, i) {
                        return true;
                    }
                    p = self.elements[i].parent;
                }
                false
            }
            Comb::Adjacent => e
                .prev_sibling
                .is_some_and(|s| self.match_from(sel, k - 1, s)),
            Comb::Sibling => {
                let mut s = e.prev_sibling;
                while let Some(i) = s {
                    if self.match_from(sel, k - 1, i) {
                        return true;
                    }
                    s = self.elements[i].prev_sibling;
                }
                false
            }
        }
    }

    fn compound(&self, c: &Compound, el: usize) -> bool {
        let e = &self.elements[el];
        c.tag.as_ref().is_none_or(|t| t == "*" || *t == e.tag)
            && c.ids.iter().all(|id| e.id.as_deref() == Some(id))
            && c.classes.iter().all(|cl| e.classes.contains(cl))
            && c.attrs.iter().all(|a| a.matches(&e.attrs))
            && c.nots.iter().all(|n| !self.compound(n, el))
    }
}

struct Html<'a> {
    src: &'a [u8],
    path: &'a str,
    out: &'a mut Markup,
}

impl Html<'_> {
    fn text(&self, n: Node) -> String {
        n.utf8_text(self.src).unwrap_or_default().to_string()
    }

    /// Records every element below `node` as a child of `parent`.
    fn children(&mut self, node: Node, parent: Option<usize>) {
        let mut prev = None;
        let mut c = node.walk();
        for child in node.children(&mut c) {
            match child.kind() {
                "element" | "script_element" | "style_element" => {
                    let mut cc = child.walk();
                    let Some(tag) = child
                        .children(&mut cc)
                        .find(|n| matches!(n.kind(), "start_tag" | "self_closing_tag"))
                    else {
                        self.children(child, parent);
                        continue;
                    };
                    let i = self.element(tag, parent, prev);
                    prev = Some(i);
                    if child.kind() == "style_element" {
                        self.style(child);
                    }
                    self.children(child, Some(i));
                }
                "self_closing_tag" => {
                    prev = Some(self.element(child, parent, prev));
                }
                _ => self.children(child, parent),
            }
        }
    }

    fn element(&mut self, tag: Node, parent: Option<usize>, prev: Option<usize>) -> usize {
        let mut name = String::new();
        let mut attrs = Vec::new();
        let mut c = tag.walk();
        for n in tag.named_children(&mut c) {
            match n.kind() {
                "tag_name" => name = self.text(n).to_lowercase(),
                "attribute" => {
                    let mut ac = n.walk();
                    let mut key = String::new();
                    let mut value = String::new();
                    for a in n.named_children(&mut ac) {
                        match a.kind() {
                            "attribute_name" => key = self.text(a).to_lowercase(),
                            "attribute_value" => value = self.text(a),
                            "quoted_attribute_value" => {
                                value = a.named_child(0).map(|v| self.text(v)).unwrap_or_default()
                            }
                            _ => {}
                        }
                    }
                    attrs.push((key, value));
                }
                _ => {}
            }
        }
        let get = |k: &str| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        // Template syntax (`{{ x }}`, `{% if %}`) is dynamic: keep only literal tokens.
        let literal = |t: &str| !t.contains(['{', '}', '%']);
        let id = get("id").filter(|v| literal(v) && !v.is_empty());
        let classes = get("class")
            .map(|v| {
                strip_templates(&v)
                    .split_whitespace()
                    .filter(|t| literal(t))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let is_sheet = get("rel").is_some_and(|r| {
            r.split_whitespace()
                .any(|t| t.eq_ignore_ascii_case("stylesheet"))
        });
        if name == "link" && is_sheet {
            if let Some(href) = get("href") {
                self.out
                    .page_refs
                    .push((self.path.to_string(), SheetRef::Href(href)));
            }
        }
        self.out.elements.push(Element {
            path: self.path.to_string(),
            line: tag.start_position().row + 1,
            tag: name,
            id,
            classes,
            attrs,
            parent,
            prev_sibling: prev,
        });
        self.out.elements.len() - 1
    }
}

impl Html<'_> {
    /// A `<style>` block: a stylesheet of its own, in document order among the links.
    fn style(&mut self, node: Node) {
        let mut c = node.walk();
        let Some(raw) = node.children(&mut c).find(|n| n.kind() == "raw_text") else {
            return;
        };
        let text = self.text(raw);
        let sheet = self.out.add_sheet(self.path);
        self.out
            .page_refs
            .push((self.path.to_string(), SheetRef::Inline(sheet)));
        let offset = raw.start_position().row;
        // Rows inside `text` start at 0; the first is on the `<style>` line itself.
        self.out.parse_css(&text, self.path, sheet, offset);
    }
}

/// `card {% if x %}active{% endif %}` → `card  active `: the tags go, the words stay.
fn strip_templates(v: &str) -> String {
    let mut out = String::new();
    let mut rest = v;
    while let Some(i) = rest.find(['{']) {
        out.push_str(&rest[..i]);
        let close = match rest[i..].get(1..2) {
            Some("%") => "%}",
            Some("{") => "}}",
            Some("#") => "#}",
            _ => {
                out.push('{');
                rest = &rest[i + 1..];
                continue;
            }
        };
        match rest[i..].find(close) {
            Some(j) => {
                out.push(' ');
                rest = &rest[i + j + 2..];
            }
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Comb {
    Descendant,
    Child,
    Adjacent,
    Sibling,
}

#[derive(Debug, Default)]
struct Compound {
    /// Specificity of pseudo-classes and pseudo-elements, which do not otherwise constrain the
    /// match (`:hover` is assumed, `:is(..)` counts its most specific argument).
    pseudo: [u32; 3],
    tag: Option<String>,
    ids: Vec<String>,
    classes: Vec<String>,
    attrs: Vec<AttrSel>,
    /// Single-compound `:not(..)` arguments.
    nots: Vec<Compound>,
    /// Has a state pseudo-class or a pseudo-element: see [`Rule::conditional`].
    conditional: bool,
}

#[derive(Debug)]
struct AttrSel {
    name: String,
    /// Operator (`=`, `~=`, `|=`, `^=`, `$=`, `*=`) and value; `None` tests presence.
    test: Option<(String, String)>,
}

impl AttrSel {
    fn matches(&self, attrs: &[(String, String)]) -> bool {
        let Some((_, v)) = attrs.iter().find(|(n, _)| *n == self.name) else {
            return false;
        };
        let Some((op, want)) = &self.test else {
            return true;
        };
        match op.as_str() {
            "=" => v == want,
            "~=" => v.split_whitespace().any(|t| t == want),
            "|=" => v == want || v.starts_with(&format!("{want}-")),
            "^=" => v.starts_with(want.as_str()),
            "$=" => v.ends_with(want.as_str()),
            "*=" => v.contains(want.as_str()),
            _ => false,
        }
    }
}

#[derive(Debug)]
struct Selector {
    compounds: Vec<Compound>,
    /// `combinators[i]` joins `compounds[i]` and `compounds[i + 1]`.
    combinators: Vec<Comb>,
}

impl Compound {
    fn specificity(&self) -> [u32; 3] {
        let tag = u32::from(self.tag.as_deref().is_some_and(|t| t != "*"));
        [
            self.ids.len() as u32 + self.pseudo[0],
            (self.classes.len() + self.attrs.len()) as u32 + self.pseudo[1],
            tag + self.pseudo[2],
        ]
    }
}

impl Selector {
    /// (ids, classes/attributes/pseudo-classes, types/pseudo-elements).
    fn specificity(&self) -> [u32; 3] {
        self.compounds.iter().fold([0; 3], |acc, c| {
            let s = c.specificity();
            [acc[0] + s[0], acc[1] + s[1], acc[2] + s[2]]
        })
    }
}

/// The most specific selector of a list (`:is(a, #b)` counts as `#b`).
fn max_specificity(list: &str) -> [u32; 3] {
    split_list(list)
        .iter()
        .filter_map(|s| parse(s))
        .map(|s| s.specificity())
        .max()
        .unwrap_or_default()
}

/// Split a selector list on top-level commas.
fn split_list(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in s.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
                continue;
            }
            _ => {}
        }
        cur.push(ch);
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// Parse one complex selector (`#main > .card a`). `None` for syntax it cannot read.
fn parse(s: &str) -> Option<Selector> {
    let mut compounds = Vec::new();
    let mut combinators = Vec::new();
    let mut cur = String::new();
    let mut pending: Option<Comb> = None;
    let mut depth = 0i32;
    let flush = |cur: &mut String,
                 pending: &mut Option<Comb>,
                 compounds: &mut Vec<Compound>,
                 combinators: &mut Vec<Comb>|
     -> Option<()> {
        if cur.is_empty() {
            return Some(());
        }
        if !compounds.is_empty() {
            combinators.push(pending.take().unwrap_or(Comb::Descendant));
        }
        compounds.push(parse_compound(cur)?);
        cur.clear();
        Some(())
    };
    for ch in s.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            _ => {}
        }
        if depth == 0 && matches!(ch, '>' | '+' | '~' | ' ' | '\n' | '\t') {
            flush(&mut cur, &mut pending, &mut compounds, &mut combinators)?;
            match ch {
                '>' => pending = Some(Comb::Child),
                '+' => pending = Some(Comb::Adjacent),
                '~' => pending = Some(Comb::Sibling),
                _ => {}
            }
            continue;
        }
        cur.push(ch);
    }
    flush(&mut cur, &mut pending, &mut compounds, &mut combinators)?;
    if compounds.is_empty() || pending.is_some() {
        return None;
    }
    Some(Selector {
        compounds,
        combinators,
    })
}

/// Pseudo-classes that hold only while the user interacts or a form control changes.
const STATES: &[&str] = &[
    "hover",
    "focus",
    "focus-within",
    "focus-visible",
    "active",
    "visited",
    "target",
    "checked",
    "indeterminate",
    "valid",
    "invalid",
    "user-invalid",
    "placeholder-shown",
    "open",
];

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '_' || c == '\\'
}

fn parse_compound(s: &str) -> Option<Compound> {
    let mut c = Compound::default();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let name = |i: &mut usize| -> String {
        let start = *i;
        while *i < chars.len() && is_name_char(chars[*i]) {
            *i += 1;
        }
        chars[start..*i].iter().collect()
    };
    while i < chars.len() {
        match chars[i] {
            '*' => {
                c.tag = Some("*".into());
                i += 1;
            }
            '#' => {
                i += 1;
                c.ids.push(name(&mut i));
            }
            '.' => {
                i += 1;
                c.classes.push(name(&mut i));
            }
            '[' => {
                let end = i + chars[i..].iter().position(|&ch| ch == ']')?;
                c.attrs
                    .push(parse_attr(&chars[i + 1..end].iter().collect::<String>()));
                i = end + 1;
            }
            ':' => {
                let element = chars.get(i + 1) == Some(&':');
                i += if element { 2 } else { 1 };
                let pname = name(&mut i).to_lowercase();
                let mut arg = None;
                if chars.get(i) == Some(&'(') {
                    let mut depth = 0;
                    let start = i + 1;
                    while i < chars.len() {
                        match chars[i] {
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                    arg = Some(chars[start..i.min(chars.len())].iter().collect::<String>());
                    i += 1;
                }
                const LEGACY_ELEMENTS: &[&str] = &["before", "after", "first-line", "first-letter"];
                match (pname.as_str(), &arg) {
                    _ if element || LEGACY_ELEMENTS.contains(&pname.as_str()) => {
                        c.pseudo[2] += 1;
                        c.conditional = true;
                    }
                    ("where", _) => {}
                    ("not" | "is" | "matches" | "has", Some(a)) => {
                        let s = max_specificity(a);
                        for (p, v) in c.pseudo.iter_mut().zip(s) {
                            *p += v;
                        }
                    }
                    _ => {
                        c.pseudo[1] += 1;
                        c.conditional |= STATES.contains(&pname.as_str());
                    }
                }
                if pname == "not" && !element {
                    if let Some(n) = arg.as_deref().and_then(|a| {
                        let list = split_list(a);
                        (list.len() == 1)
                            .then(|| parse_compound(&list[0]))
                            .flatten()
                    }) {
                        c.nots.push(n);
                    }
                }
            }
            ch if is_name_char(ch) => c.tag = Some(name(&mut i).to_lowercase()),
            _ => return None,
        }
    }
    Some(c)
}

fn parse_attr(s: &str) -> AttrSel {
    for op in ["~=", "|=", "^=", "$=", "*=", "="] {
        if let Some((n, v)) = s.split_once(op) {
            let v = v.trim();
            // A trailing ` i` / ` s` flag.
            let v = v
                .strip_suffix(" i")
                .or_else(|| v.strip_suffix(" s"))
                .unwrap_or(v)
                .trim();
            return AttrSel {
                name: n.trim().to_lowercase(),
                test: Some((
                    op.to_string(),
                    v.trim_matches(|c| c == '"' || c == '\'').to_string(),
                )),
            };
        }
    }
    AttrSel {
        name: s.trim().to_lowercase(),
        test: None,
    }
}
