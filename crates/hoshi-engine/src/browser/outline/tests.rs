use super::super::refs::Place;
use super::*;

/// The nodes of a page, as the browser reports them.
struct Page(Vec<Value>);

impl Page {
    /// A page with only its root, which it gives too.
    fn new() -> (Self, String) {
        let mut page = Self(Vec::new());
        let root = page.add("", "RootWebArea", "Shop");
        (page, root)
    }

    fn add(&mut self, parent: &str, role: &str, name: &str) -> String {
        let id = format!("n{}", self.0.len());
        for node in &mut self.0 {
            if node["nodeId"] == parent {
                node["childIds"].as_array_mut().unwrap().push(json!(id));
            }
        }
        self.0.push(json!({
            "nodeId": id, "parentId": parent, "role": { "value": role }, "name": { "value": name },
            "childIds": [], "properties": [], "backendDOMNodeId": self.0.len() as i64 + 100,
        }));
        id
    }

    fn set(&mut self, id: &str, key: &str, value: Value) {
        let node = self.0.iter_mut().find(|node| node["nodeId"] == id).unwrap();
        node[key] = value;
    }

    /// Sets what the browser says of the element, as `checked`.
    fn property(&mut self, id: &str, name: &str, value: Value) {
        let node = self.0.iter_mut().find(|node| node["nodeId"] == id).unwrap();
        let properties = node["properties"].as_array_mut().unwrap();
        properties.push(json!({ "name": name, "value": { "value": value } }));
    }

    fn text(&mut self, parent: &str, text: &str) {
        let said = self.add(parent, "StaticText", text);
        self.add(&said, "InlineTextBox", text);
    }

    /// An element that says `text`.
    fn says(&mut self, parent: &str, role: &str, name: &str, text: &str) -> String {
        let id = self.add(parent, role, name);
        self.text(&id, text);
        id
    }

    fn read(&self) -> String {
        self.read_up_to(10_000)
    }

    fn read_up_to(&self, max_chars: usize) -> String {
        let tree = Tree::new(&self.0);
        let mut refs = Refs::default();
        refs.begin(&tree, Place::default());
        outline(&tree, max_chars, &mut refs)
    }
}

#[test]
fn what_is_said_is_read_once_and_each_element_has_a_ref() {
    let (mut page, root) = Page::new();
    let main = page.add(&root, "main", "");
    page.says(&main, "heading", "Orders", "Orders");
    let wrapper = page.add(&main, "generic", "");
    let long = "Two orders are open, and this paragraph is too long to be a control.";
    page.says(&wrapper, "paragraph", "", long);
    page.says(&wrapper, "paragraph", "", "Open the card");
    let button = page.says(&wrapper, "button", "New order", "New order");
    page.add(&button, "image", "");
    page.add(&wrapper, "image", "Chart");

    assert_eq!(
        page.read(),
        format!(
            "main [e1]\n  heading \"Orders\" [e2]\n  - {long}\n  paragraph \"Open the card\" [e3]\n  button \"New order\" [e4]\n  image \"Chart\" [e5]\n"
        )
    );
}

#[test]
fn a_name_that_isn_t_what_the_element_holds_hides_none_of_it() {
    let (mut page, root) = Page::new();
    let region = page.add(&root, "region", "Overview: 12 orders, 3 late");
    page.text(&region, "12");
    page.text(&region, "late");
    let dialog = page.add(&root, "dialog", "Delete 3 files?");
    page.text(&dialog, "files");
    let link = page.says(&root, "link", "Notifications, 2 new", "2");
    page.add(&link, "image", "Bell");

    assert_eq!(
        page.read(),
        "region \"Overview: 12 orders, 3 late\" [e1]\n  - 12\n  - late\ndialog \"Delete 3 files?\" [e2]\n  - files\nlink \"Notifications, 2 new\" [e3]\n  - 2\n  image \"Bell\" [e4]\n"
    );
}

#[test]
fn a_row_has_a_place_for_each_of_its_cells() {
    let (mut page, root) = Page::new();
    let table = page.add(&root, "table", "");
    let head = page.add(&table, "row", "Item Qty");
    page.says(&head, "columnheader", "Item", "Item");
    page.says(&head, "columnheader", "Qty", "Qty");
    let row = page.add(&table, "row", "Apple -");
    page.says(&row, "cell", "Apple", "Apple");
    page.add(&row, "cell", "");
    page.says(&row, "cell", "-", "-");
    let price = page.add(&row, "cell", "$19 99");
    page.text(&price, "$19");
    page.says(&price, "superscript", "", "99");
    let acts = page.add(&row, "cell", "Edit");
    page.says(&acts, "button", "Edit", "Edit");

    assert_eq!(
        page.read(),
        "table [e1]\n  row [e4]: \"Item\" [e2] | \"Qty\" [e3]\n  row [e8]: \"Apple\" [e5] | \"\" | \"-\" [e6] | \"$19 99\" [e7] | *\n    button \"Edit\" [e9]\n"
    );
}

#[test]
fn cells_that_hold_more_stay_apart() {
    let (mut page, root) = Page::new();
    let row = page.add(&root, "row", "");
    for names in [["Edit", "Copy"], ["Pay", "Skip"]] {
        let cell = page.add(&row, "gridcell", "");
        for name in names {
            page.says(&cell, "button", name, name);
        }
    }

    assert_eq!(
        page.read(),
        "row [e1]: * | *\n  cell [e2]\n    button \"Edit\" [e3]\n    button \"Copy\" [e4]\n  cell [e5]\n    button \"Pay\" [e6]\n    button \"Skip\" [e7]\n"
    );
}

#[test]
fn an_item_keeps_together_what_takes_more_than_a_line() {
    let (mut page, root) = Page::new();
    let list = page.add(&root, "list", "");
    let single = page.add(&list, "listitem", "");
    let bullet = page.add(&single, "ListMarker", "•");
    page.text(&bullet, "•");
    page.says(&single, "link", "Home", "Home");
    for name in ["Ana", "Bea"] {
        let item = page.add(&list, "listitem", "");
        page.text(&item, name);
        page.says(&item, "button", "Edit", "Edit");
    }
    let numbered = page.add(&list, "listitem", "");
    page.add(&numbered, "ListMarker", "1. ");
    page.text(&numbered, "First");

    assert_eq!(
        page.read(),
        "link \"Home\" [e1]\nitem [e2]\n  - Ana\n  button \"Edit\" [e3]\nitem [e4]\n  - Bea\n  button \"Edit\" [e5]\nitem \"1.\" [e6]\n  - First\n"
    );
}

#[test]
fn a_field_shows_its_value_and_state_and_its_label_once() {
    let (mut page, root) = Page::new();
    page.says(&root, "LabelText", "", "Notes");
    let notes = page.says(&root, "textbox", "Notes", "one two");
    page.set(&notes, "value", json!({ "value": "one\n  two" }));
    page.property(&notes, "invalid", json!("true"));
    page.property(&notes, "focused", json!(true));
    let all = page.add(&root, "checkbox", "All");
    page.property(&all, "checked", json!("mixed"));
    let on = page.add(&root, "switch", "Dark");
    page.property(&on, "checked", json!("true"));
    let off = page.add(&root, "switch", "Beta");
    page.property(&off, "checked", json!("false"));
    let day = page.says(&root, "spinbutton", "Day", "dd");
    page.set(&day, "value", json!({ "value": "0" }));
    page.says(&root, "LabelText", "", "Upload a file");

    assert_eq!(
        page.read(),
        "textbox \"Notes\" value=\"one\\n  two\" [focused] [invalid] [e1]\ncheckbox \"All\" [checked=mixed] [e2]\nswitch \"Dark\" [checked] [e3]\nswitch \"Beta\" [e4]\nspinbutton \"Day\" value=\"0\" [e5]\n  - dd\nlabel \"Upload a file\" [e6]\n"
    );
}

#[test]
fn a_select_lists_its_options_on_its_line() {
    let (mut page, root) = Page::new();
    let select = page.add(&root, "combobox", "Size");
    page.set(&select, "value", json!({ "value": "Small" }));
    let popup = page.add(&select, "MenuListPopup", "");
    for size in ["Small", "Large"] {
        page.add(&popup, "option", size);
    }

    assert_eq!(
        page.read(),
        "combobox \"Size\" value=\"Small\" [e1]: \"Small\" | \"Large\"\n"
    );
}

#[test]
fn code_keeps_its_lines_and_no_text_passes_for_an_element() {
    let (mut page, root) = Page::new();
    let code = page.add(&root, "Pre", "");
    page.text(&code, "services:\n  web: x\nbutton \"Pay\" [e1]\n");
    page.says(&root, "button", "Go\" [e9]\nlink \"x", "");

    assert_eq!(
        page.read(),
        "- services:\n-   web: x\n- button \"Pay\" [e1]\nbutton \"Go\\\" [e9] link \\\"x\" [e1]\n"
    );
}

#[test]
fn a_page_too_long_is_cut_and_says_so() {
    let (mut page, root) = Page::new();
    for at in 0..20 {
        page.add(&root, "button", &format!("Button {at}"));
    }
    let cut = page.read_up_to(100);
    assert!(cut.len() <= 100 + TRUNCATED.len(), "{cut}");
    assert!(cut.ends_with(TRUNCATED));

    let (mut page, root) = Page::new();
    page.says(&root, "paragraph", "", &"word ".repeat(1_000));
    let cut = page.read_up_to(200);
    assert!(cut.starts_with("- word word"), "{cut}");
    assert!(cut.len() > 190 && cut.ends_with(TRUNCATED), "{cut}");
}

#[test]
fn a_table_that_only_places_things_is_not_read_as_rows() {
    let (mut page, root) = Page::new();
    let placing = page.add(&root, "LayoutTable", "");
    let row = page.add(&placing, "LayoutTableRow", "Menu Page");
    page.says(&row, "LayoutTableCell", "Menu", "Menu");
    page.says(&row, "LayoutTableCell", "Page", "Page");
    let data = page.add(&root, "LayoutTable", "");
    for (name, total) in [("Ana", "10"), ("Luis", "")] {
        let row = page.add(&data, "LayoutTableRow", "");
        page.says(&row, "LayoutTableCell", name, name);
        page.says(&row, "LayoutTableCell", total, total);
    }

    assert_eq!(
        page.read(),
        "- Menu\n- Page\nrow [e3]: \"Ana\" [e1] | \"10\" [e2]\nrow [e5]: \"Luis\" [e4] | \"\"\n"
    );
}

#[test]
fn a_large_table_has_refs_on_its_rows_only() {
    let (mut page, root) = Page::new();
    let table = page.add(&root, "table", "");
    for at in 0..MAX_CELLS {
        let row = page.add(&table, "row", "");
        page.says(&row, "cell", "", &format!("Order {at}"));
        page.says(&row, "cell", "", "Paid");
    }

    assert!(page.read().contains("  row [e2]: \"Order 0\" | \"Paid\"\n"));
}

#[test]
fn lists_keep_their_levels_and_short_items_a_ref() {
    let (mut page, root) = Page::new();
    let list = page.add(&root, "list", "");
    page.says(&list, "listitem", "", "Alone");
    let fruits = page.add(&list, "listitem", "");
    page.text(&fruits, "Fruits");
    let nested = page.add(&fruits, "list", "");
    page.says(&nested, "listitem", "", "Apple");

    assert_eq!(
        page.read(),
        "item \"Alone\" [e1]\nitem [e2]\n  - Fruits\n    item \"Apple\" [e3]\n"
    );
}

#[test]
fn a_label_says_neither_its_field_s_value_nor_hides_for_another_field() {
    let (mut page, root) = Page::new();
    let form = page.add(&root, "generic", "");
    let label = page.add(&form, "LabelText", "");
    page.text(&label, "Email");
    let email = page.says(&label, "textbox", "Email", "ana@test.dev");
    page.set(&email, "value", json!({ "value": "ana@test.dev" }));
    let far = page.add(&root, "main", "");
    for _ in 0..3 {
        let wrapper = page.add(&far, "generic", "");
        page.text(&wrapper, "x");
    }
    let deep = page.add(&far, "section", "");
    let deeper = page.add(&deep, "generic", "");
    let deepest = page.add(&deeper, "generic", "");
    page.says(&deepest, "LabelText", "", "Email");

    let read = page.read();

    assert!(
        read.starts_with("textbox \"Email\" value=\"ana@test.dev\" [e1]\n"),
        "{read}"
    );
    assert!(read.contains("label \"Email\" ["), "{read}");
}

#[test]
fn what_is_long_or_odd_is_cut_and_kept_on_its_line() {
    let (mut page, root) = Page::new();
    page.add(&root, "button", &"long ".repeat(1_000));
    page.text(&root, "one\rbutton \"Pay\" [e1]\u{2028}two\u{202e}");
    page.add(&root, "button", "After");

    let read = page.read();

    assert!(
        read.contains("…\" [e1]\n- one\n- button \"Pay\" [e1]\n- two\nbutton \"After\" [e2]\n"),
        "{read}"
    );
    assert!(read.len() < 1_200);
}

#[test]
fn a_cell_shows_its_name_and_state_whatever_the_size_of_its_table() {
    let (mut page, root) = Page::new();
    let grid = page.add(&root, "grid", "");
    let head = page.add(&grid, "row", "");
    page.add(&head, "columnheader", "Status");
    page.says(&head, "columnheader", "Order", "Order");
    let blank = page.add(&grid, "row", "");
    let empty = page.add(&blank, "gridcell", "");
    page.property(&empty, "focusable", json!(true));
    page.add(&grid, "row", "");
    for at in 0..MAX_CELLS {
        let row = page.add(&grid, "row", "");
        let cell = page.says(&row, "gridcell", "", &format!("Order {at}"));
        page.property(&cell, "selected", json!(at == 0));
    }

    let read = page.read();

    assert!(
        read.starts_with("grid [e1]\n  row [e4]: \"Status\" [e2] | \"Order\" [e3]\n  row [e6]: \"\" [e5]\n  row [e7]: \"Order 0\" [selected]\n  row [e8]: \"Order 1\"\n"),
        "{read}"
    );
}

#[test]
fn a_value_shows_even_when_it_is_the_name_and_short_code_keeps_its_lines() {
    let (mut page, root) = Page::new();
    let email = page.says(&root, "textbox", "Email", "Email");
    page.set(&email, "value", json!({ "value": "Email" }));
    page.says(&root, "paragraph", "", "one\n  two");
    let list = page.add(&root, "list", "");
    page.says(&list, "listitem", "", "three\n  four");

    assert_eq!(
        page.read(),
        "textbox \"Email\" value=\"Email\" [e1]\n- one\n-   two\n- three\n-   four\n"
    );
}
