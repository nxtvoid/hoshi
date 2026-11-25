//! A page as an agent reads it: its elements as `button "Save" [e3]`, with their refs, and its
//! text as `- text`, each said once. What only groups the rest is left out. A name is quoted,
//! so no text of the page passes for an element.

use super::refs::{Refs, Tree, name_of, role_of, text_of};
use serde_json::{Value, json};
use std::collections::HashMap;

/// Roles that say nothing their text doesn't, unless they carry a name of their own.
const PLAIN_ROLES: [&str; 42] = [
    "Abbr",
    "banner",
    "blockquote",
    "caption",
    "code",
    "complementary",
    "contentinfo",
    "definition",
    "deletion",
    "DescriptionList",
    "DescriptionListDetail",
    "DescriptionListTerm",
    "emphasis",
    "Figcaption",
    "figure",
    "FooterAsNonLandmark",
    "generic",
    "group",
    "HeaderAsNonLandmark",
    "insertion",
    "LayoutTable",
    "LayoutTableBody",
    "Legend",
    "list",
    "log",
    "mark",
    "none",
    "paragraph",
    "Pre",
    "presentation",
    "region",
    "rowgroup",
    "section",
    "sectionfooter",
    "sectionheader",
    "status",
    "strong",
    "subscript",
    "superscript",
    "SvgRoot",
    "term",
    "time",
];
const CELL_ROLES: [&str; 5] = [
    "cell",
    "columnheader",
    "gridcell",
    "LayoutTableCell",
    "rowheader",
];
/// Roles the user acts on.
const ACTED_ROLES: [&str; 16] = [
    "button",
    "checkbox",
    "combobox",
    "Iframe",
    "link",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "option",
    "radio",
    "searchbox",
    "slider",
    "spinbutton",
    "switch",
    "tab",
    "textbox",
];
/// Roles a label names, which show the name on their own line.
const LABELED_ROLES: [&str; 15] = [
    "checkbox",
    "ColorWell",
    "combobox",
    "Date",
    "DateTime",
    "InputTime",
    "listbox",
    "meter",
    "progressbar",
    "radio",
    "searchbox",
    "slider",
    "spinbutton",
    "switch",
    "textbox",
];
/// Roles whose text is their value.
const FIELD_ROLES: [&str; 4] = ["textbox", "searchbox", "spinbutton", "combobox"];
const FLAGS: [&str; 9] = [
    "checked", "disabled", "expanded", "selected", "pressed", "focused", "invalid", "required",
    "readonly",
];
/// A paragraph or item up to this long may be a control of the page's own making, so it has
/// a ref.
const MAX_SHORT: usize = 60;
/// The cells of a table up to this many have refs, as those of a calendar.
const MAX_CELLS: usize = 120;
/// What is quoted of a name, a value or a cell.
const MAX_QUOTED: usize = 1_000;
/// How far around a label the field it names is looked for.
const MAX_LABEL_UP: usize = 3;
const MAX_LABEL_LOOK: usize = 200;
const MAX_OPTIONS: usize = 30;
const TRUNCATED: &str = "[truncated: the page has more]\n";

fn plain(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether a character starts a new line, so text with it could pass for another line.
fn is_break(one: char) -> bool {
    matches!(
        one,
        '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// `text` without what changes the order or the place its characters are shown in.
fn harmless(text: &str) -> String {
    text.chars()
        .filter(|one| {
            !matches!(one, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                && (!one.is_control() || is_break(*one) || *one == '\t')
        })
        .collect()
}

/// `text` in quotes, cut when it is long.
fn quoted(text: &str) -> String {
    let text = harmless(text).replace(is_break, "\n");
    let cut = match text.char_indices().nth(MAX_QUOTED) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text,
    };
    json!(cut).to_string()
}

fn squeezed(text: &str) -> String {
    text.split_whitespace().collect()
}

fn is_text(node: &Value) -> bool {
    node["role"]["value"] == "StaticText"
}

/// Nodes that only lay out the text of others.
fn is_layout(role: &str) -> bool {
    matches!(role, "InlineTextBox" | "LineBreak")
}

fn property<'a>(node: &'a Value, name: &str) -> Option<&'a Value> {
    node["properties"]
        .as_array()?
        .iter()
        .find(|property| property["name"] == name)
        .map(|property| &property["value"]["value"])
}

/// Whether the user can focus it, or it is one the browser always lets them use.
fn is_acted_on(node: &Value) -> bool {
    node["ignored"] != true
        && role_of(node) != "RootWebArea"
        && (property(node, "focusable").is_some_and(|value| value == true)
            || ACTED_ROLES.contains(&role_of(node).as_str()))
}

fn flags(node: &Value) -> String {
    let mut flags = String::new();
    for flag in FLAGS {
        match property(node, flag) {
            Some(Value::Bool(true)) => flags.push_str(&format!(" [{flag}]")),
            Some(Value::String(value)) if value == "true" => {
                flags.push_str(&format!(" [{flag}]"));
            }
            Some(Value::String(value)) if !matches!(value.as_str(), "false" | "") => {
                flags.push_str(&format!(" [{flag}={value}]"));
            }
            _ => {}
        }
    }
    flags
}

/// The outline of the page `tree` shows, up to `max_chars`.
pub(super) fn outline(tree: &Tree, max_chars: usize, refs: &mut Refs) -> String {
    let mut writer = Writer {
        tree,
        refs,
        tables: HashMap::new(),
        max_chars,
        out: String::new(),
        full: false,
    };
    let mut stack: Vec<Step> = tree
        .root()
        .map(|node| Step::new(node, 0))
        .into_iter()
        .collect();
    while let Some(step) = stack.pop() {
        let next = writer.node(&step);
        if writer.full {
            break;
        }
        stack.extend(next.into_iter().rev());
    }
    writer.out
}

/// A node to write; what it says is already told by the line it is under when `told`.
struct Step<'a> {
    node: &'a Value,
    depth: usize,
    told: bool,
    /// Whether a cell has a line of its own.
    headed: bool,
}

impl<'a> Step<'a> {
    fn new(node: &'a Value, depth: usize) -> Self {
        Self {
            node,
            depth,
            told: false,
            headed: false,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Table {
    cells: usize,
    /// Whether it has rows and columns, rather than one of them to place what it holds.
    lists: bool,
}

struct Writer<'a, 't> {
    tree: &'t Tree<'a>,
    refs: &'t mut Refs,
    /// The shape of each table, by its node.
    tables: HashMap<*const Value, Table>,
    max_chars: usize,
    out: String,
    full: bool,
}

impl<'a> Writer<'a, '_> {
    /// Writes a line, or as much of it as there is room for.
    fn line(&mut self, depth: usize, text: &str) {
        if self.full {
            return;
        }
        let line = format!("{}{text}\n", "  ".repeat(depth));
        let room = self.max_chars.saturating_sub(self.out.len());
        if line.len() <= room {
            self.out.push_str(&line);
            return;
        }
        self.full = true;
        let cut = &line[..line.floor_char_boundary(room)];
        if text.starts_with("- ") && cut.trim().len() > 20 {
            self.out.push_str(cut);
            self.out.push_str("…\n");
        }
        self.out.push_str(TRUNCATED);
    }

    fn give(&mut self, node: &Value) -> String {
        self.refs
            .give(self.tree, node)
            .map(|reference| format!(" [{reference}]"))
            .unwrap_or_default()
    }

    /// The shape of the table a row is in.
    fn table(&mut self, row: &'a Value) -> Table {
        let mut table = row;
        while let Some(parent) = self.tree.parent(table) {
            table = parent;
            if matches!(
                role_of(table).as_str(),
                "table" | "grid" | "treegrid" | "LayoutTable"
            ) {
                break;
            }
        }
        let key = std::ptr::from_ref(table);
        if let Some(known) = self.tables.get(&key) {
            return *known;
        }
        let (mut rows, mut columns, mut shape) = (0, 0, Table::default());
        let mut stack: Vec<&Value> = self.tree.children(table).collect();
        while let Some(current) = stack.pop() {
            let role = role_of(current);
            if matches!(role.as_str(), "row" | "LayoutTableRow") && current["ignored"] != true {
                let cells = self.cells(current).0.len();
                rows += 1;
                columns = columns.max(cells);
                shape.cells += cells;
            } else if role != "LayoutTable" && role != "table" {
                stack.extend(self.tree.children(current));
            }
        }
        shape.lists = rows > 1 && columns > 1;
        self.tables.insert(key, shape);
        shape
    }

    /// The cells of a row, and what else is in it.
    fn cells(&self, row: &'a Value) -> (Vec<&'a Value>, Vec<&'a Value>) {
        let (mut cells, mut rest) = (Vec::new(), Vec::new());
        let mut stack: Vec<&Value> = self.tree.children(row).rev().collect();
        while let Some(current) = stack.pop() {
            if current["ignored"] == true {
                stack.extend(self.tree.children(current).rev());
            } else if CELL_ROLES.contains(&role_of(current).as_str()) {
                cells.push(current);
            } else {
                rest.push(current);
            }
        }
        (cells, rest)
    }

    fn is_row(&mut self, node: &'a Value) -> bool {
        match role_of(node).as_str() {
            "row" => true,
            "LayoutTableRow" => self.table(node).lists,
            _ => false,
        }
    }

    /// What a label says, without what is in the fields it holds.
    fn label_text(&self, node: &'a Value) -> String {
        let mut texts = Vec::new();
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            let role = role_of(current);
            if role == "StaticText" {
                texts.push(plain(&text_of(&current["name"]["value"])));
            } else if !is_layout(&role) && !LABELED_ROLES.contains(&role.as_str()) {
                stack.extend(self.tree.children(current).rev());
            }
        }
        plain(&texts.join(" "))
    }

    /// Whether a field near the label shows its name.
    fn names_a_field(&self, label: &'a Value, text: &str) -> bool {
        let mut around = label;
        for _ in 0..MAX_LABEL_UP {
            let named = self
                .tree
                .descendants(around)
                .take(MAX_LABEL_LOOK)
                .filter(|node| node["ignored"] != true && plain(&name_of(node)) == text)
                .any(|node| LABELED_ROLES.contains(&role_of(node).as_str()));
            if named {
                return true;
            }
            let Some(parent) = self.tree.parent(around) else {
                break;
            };
            around = parent;
        }
        false
    }

    /// What is said in the node, piece by piece, without the markers of lists.
    fn texts(&self, node: &'a Value) -> Vec<String> {
        let mut texts = Vec::new();
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            let role = role_of(current);
            if role == "StaticText" {
                let text = plain(&text_of(&current["name"]["value"]));
                if !text.is_empty() {
                    texts.push(text);
                }
            } else if role != "ListMarker" && !is_layout(&role) {
                stack.extend(self.tree.children(current).rev());
            }
        }
        texts
    }

    /// Whether the node is named by what it holds, so its name tells that.
    fn is_named_by_contents(&self, node: &'a Value) -> bool {
        let name = squeezed(&name_of(node));
        if name.is_empty() {
            return false;
        }
        let mut said = String::new();
        let mut stack: Vec<&Value> = self.tree.children(node).rev().collect();
        while let Some(current) = stack.pop() {
            let role = role_of(current);
            let own = squeezed(&name_of(current));
            if role == "StaticText" || role == "image" && !own.is_empty() {
                said.push_str(&own);
                if said.len() > name.len() {
                    return false;
                }
            } else if !is_layout(&role) && role != "ListMarker" {
                stack.extend(self.tree.children(current).rev());
            }
        }
        said == name
    }

    /// Whether the node has a line of its own.
    fn is_shown(&self, node: &'a Value) -> bool {
        let role = role_of(node);
        let grouping = name_of(node).is_empty() && PLAIN_ROLES.contains(&role.as_str())
            || role == "LayoutTableRow"
            || CELL_ROLES.contains(&role.as_str())
            || role == "listitem"
            || role == "row";
        node["ignored"] != true
            && role != "RootWebArea"
            && role != "StaticText"
            && role != "ListMarker"
            && !is_layout(&role)
            && (is_acted_on(node) || !grouping)
    }

    /// Whether what the node holds takes more than one line.
    fn takes_lines(&self, node: &'a Value) -> bool {
        let mut lines = 0;
        let mut stack: Vec<&Value> = self.tree.children(node).rev().collect();
        while let Some(current) = stack.pop() {
            let role = role_of(current);
            if role == "ListMarker" || is_layout(&role) {
                continue;
            }
            let text = is_text(current) && !name_of(current).is_empty();
            let shown = self.is_shown(current);
            if text || shown || role == "row" {
                lines += 1;
                if lines > 1 {
                    return true;
                }
            }
            if !text && !(shown && self.is_named_by_contents(current)) {
                stack.extend(self.tree.children(current).rev());
            }
        }
        false
    }

    fn steps(&self, node: &'a Value, depth: usize, told: bool) -> Vec<Step<'a>> {
        self.tree
            .children(node)
            .map(|node| Step {
                told,
                ..Step::new(node, depth)
            })
            .collect()
    }

    /// Writes what the node says itself, and gives what is left to write in it.
    fn node(&mut self, step: &Step<'a>) -> Vec<Step<'a>> {
        let Step {
            node,
            depth,
            told,
            headed,
        } = *step;
        let role = role_of(node);
        if is_layout(&role) || role == "ListMarker" {
            return Vec::new();
        }
        if role == "StaticText" {
            if !told {
                self.text(node, depth);
            }
            return Vec::new();
        }
        if node["ignored"] == true || role == "RootWebArea" {
            return self.steps(node, depth, told);
        }
        match role.as_str() {
            "listitem" => self.item(node, depth, told),
            "LabelText" if !is_acted_on(node) => self.label(node, depth, told),
            "paragraph" if !told && self.short_paragraph(node, depth) => Vec::new(),
            "image" if told && !is_acted_on(node) => Vec::new(),
            _ if self.is_row(node) => self.row(node, depth),
            "list" if name_of(node).is_empty() && !is_acted_on(node) => {
                let nested = self.is_in_item(node);
                self.steps(node, depth + usize::from(nested), told)
            }
            _ if headed => {
                let (flags, reference) = (flags(node), self.give(node));
                self.line(depth, &format!("cell{flags}{reference}"));
                self.steps(node, depth + 1, false)
            }
            _ if self.is_shown(node) => self.element(node, &role, depth, told),
            _ => self.steps(node, depth, told),
        }
    }

    fn is_in_item(&self, node: &'a Value) -> bool {
        let mut parent = self.tree.parent(node);
        while let Some(current) = parent.filter(|parent| parent["ignored"] == true) {
            parent = self.tree.parent(current);
        }
        parent.is_some_and(|parent| role_of(parent) == "listitem")
    }

    /// Text as it stands, line by line.
    fn text(&mut self, node: &Value, depth: usize) {
        let text = harmless(&text_of(&node["name"]["value"]));
        let lines: Vec<&str> = text
            .split(is_break)
            .filter(|line| !line.trim().is_empty())
            .collect();
        match lines[..] {
            [line] => self.line(depth, &format!("- {}", line.trim())),
            _ => {
                for line in lines {
                    self.line(depth, &format!("- {}", line.trim_end()));
                }
            }
        }
    }

    fn short_paragraph(&mut self, node: &'a Value, depth: usize) -> bool {
        let texts = self.texts(node);
        let simple = !is_acted_on(node)
            && name_of(node).is_empty()
            && texts.len() == 1
            && texts[0].chars().count() <= MAX_SHORT
            && !self
                .tree
                .descendants(node)
                .any(|child| self.is_shown(child));
        if simple {
            let reference = self.give(node);
            self.line(
                depth,
                &format!("paragraph {}{reference}", quoted(&texts[0])),
            );
        }
        simple
    }

    /// A label, unless the field it names says the same.
    fn label(&mut self, node: &'a Value, depth: usize, told: bool) -> Vec<Step<'a>> {
        let text = self.label_text(node);
        let named = self.names_a_field(node, &text);
        if told || text.is_empty() || named {
            return self.steps(node, depth, told || named);
        }
        let reference = self.give(node);
        self.line(depth, &format!("label {}{reference}", quoted(&text)));
        self.steps(node, depth + 1, true)
    }

    fn item(&mut self, node: &'a Value, depth: usize, told: bool) -> Vec<Step<'a>> {
        let marker = self
            .tree
            .children(node)
            .find(|child| role_of(child) == "ListMarker")
            .map(|marker| plain(&name_of(marker)))
            .filter(|marker| marker.chars().any(char::is_alphanumeric));
        if !is_acted_on(node) && marker.is_none() && !self.takes_lines(node) {
            let texts = self.texts(node);
            let shown = self
                .tree
                .descendants(node)
                .any(|child| self.is_shown(child));
            if told || shown || texts.len() != 1 || texts[0].chars().count() > MAX_SHORT {
                return self.steps(node, depth, told);
            }
            let reference = self.give(node);
            self.line(depth, &format!("item {}{reference}", quoted(&texts[0])));
            return Vec::new();
        }
        let marker = marker
            .map(|marker| format!(" {}", quoted(&marker)))
            .unwrap_or_default();
        let (flags, reference) = (flags(node), self.give(node));
        self.line(depth, &format!("item{marker}{flags}{reference}"));
        self.steps(node, depth + 1, told)
    }

    /// A row with the text of each of its cells, `*` for those that hold more, written after.
    fn row(&mut self, node: &'a Value, depth: usize) -> Vec<Step<'a>> {
        let (cells, rest) = self.cells(node);
        let small = self.table(node).cells <= MAX_CELLS;
        let (flags_of_row, reference) = (flags(node), self.give(node));
        let mut slots = Vec::new();
        let mut held = Vec::new();
        for cell in cells {
            let holds = self
                .tree
                .descendants(cell)
                .any(|child| self.is_shown(child) || role_of(child) == "row");
            if holds {
                held.push(cell);
                slots.push("*".to_owned());
                continue;
            }
            let text = self.texts(cell).join(" ");
            let mut slot = quoted(&text);
            let acts = is_acted_on(cell) || role_of(cell) == "columnheader";
            if acts || small && !text.is_empty() {
                slot.push_str(&flags(cell));
                slot.push_str(&self.give(cell));
            }
            slots.push(slot);
        }
        let says = slots.iter().any(|slot| !slot.starts_with("\"\""));
        if !says && rest.is_empty() && !is_acted_on(node) {
            return Vec::new();
        }
        let mut line = format!("row{flags_of_row}{reference}");
        if !slots.is_empty() {
            line.push_str(&format!(": {}", slots.join(" | ")));
        }
        self.line(depth, &line);
        let apart = held.len() > 1;
        let mut next = Vec::new();
        for cell in held {
            // A cell among others, or one to act on, has a line to hold what is in it.
            if apart || is_acted_on(cell) || role_of(cell) == "columnheader" {
                next.push(Step {
                    headed: true,
                    ..Step::new(cell, depth + 1)
                });
            } else {
                next.extend(self.steps(cell, depth + 1, false));
            }
        }
        next.extend(rest.into_iter().map(|node| Step::new(node, depth + 1)));
        next
    }

    fn element(&mut self, node: &'a Value, role: &str, depth: usize, told: bool) -> Vec<Step<'a>> {
        let name = plain(&name_of(node));
        let field = FIELD_ROLES.contains(&role);
        let by_contents = !field && self.is_named_by_contents(node);
        // Told by the line it is under, and nothing to act on.
        if told && by_contents && !is_acted_on(node) {
            return self.steps(node, depth, true);
        }
        let mut line = role.to_owned();
        if !name.is_empty() {
            line.push_str(&format!(" {}", quoted(&name)));
        }
        let value = text_of(&node["value"]["value"]);
        if !value.trim().is_empty() && value != name {
            line.push_str(&format!(" value={}", quoted(&value)));
        }
        line.push_str(&flags(node));
        line.push_str(&self.give(node));
        let options = self.options(node);
        if let Some(options) = &options {
            line.push_str(&format!(": {options}"));
        }
        self.line(depth, &line);
        if options.is_some() {
            return Vec::new();
        }
        // What a field shows may not be its value, as the parts of an empty date.
        let told = by_contents || field && squeezed(&self.texts(node).concat()) == squeezed(&value);
        self.steps(node, depth + 1, told)
    }

    /// What a closed `<select>` offers, which is picked on it.
    fn options(&self, node: &'a Value) -> Option<String> {
        let popup = self
            .tree
            .children(node)
            .find(|child| role_of(child) == "MenuListPopup")?;
        let names: Vec<String> = self
            .tree
            .descendants(popup)
            .filter(|child| role_of(child) == "option")
            .map(|option| quoted(&plain(&name_of(option))))
            .collect();
        let mut shown = names[..names.len().min(MAX_OPTIONS)].join(" | ");
        if names.len() > MAX_OPTIONS {
            shown.push_str(&format!(" | {} more", names.len() - MAX_OPTIONS));
        }
        Some(shown)
    }
}

#[cfg(test)]
mod tests;
