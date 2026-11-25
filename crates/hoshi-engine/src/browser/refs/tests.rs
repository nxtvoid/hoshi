use super::*;
use serde_json::json;

/// A list of `rows`, each with its texts and a button, whose button has this node id.
fn list(rows: &[(&[&str], &str, i64)]) -> Vec<Value> {
    let mut nodes = vec![json!({
        "nodeId": "root", "role": { "value": "RootWebArea" }, "backendDOMNodeId": 1,
        "childIds": (0..rows.len()).map(|row| format!("li{row}")).collect::<Vec<_>>(),
    })];
    for (row, (texts, button, id)) in rows.iter().enumerate() {
        let mut children: Vec<String> = (0..texts.len()).map(|at| format!("t{row}-{at}")).collect();
        children.push(format!("b{row}"));
        nodes.push(json!({ "nodeId": format!("li{row}"), "parentId": "root", "role": { "value": "generic" }, "ignored": true, "childIds": children }));
        for (at, text) in texts.iter().enumerate() {
            nodes.push(json!({ "nodeId": format!("t{row}-{at}"), "parentId": format!("li{row}"), "role": { "value": "StaticText" }, "name": { "value": text }, "childIds": [] }));
        }
        nodes.push(json!({ "nodeId": format!("b{row}"), "parentId": format!("li{row}"), "role": { "value": "button" }, "name": { "value": button }, "childIds": [], "backendDOMNodeId": id }));
    }
    nodes
}

fn place(url: &str) -> Place {
    Place {
        document: "1".into(),
        url: url.into(),
    }
}

/// The refs of the buttons in `nodes`, in order.
fn read(refs: &mut Refs, nodes: &[Value]) -> Vec<String> {
    let tree = Tree::new(nodes);
    refs.begin(&tree, place("/"));
    nodes
        .iter()
        .filter(|node| node["role"]["value"] == "button")
        .map(|node| refs.give(&tree, node).unwrap())
        .collect()
}

fn find(refs: &Refs, reference: &str, nodes: &[Value]) -> Option<i64> {
    refs.find(reference, &Tree::new(nodes), &place("/")).ok()
}

#[test]
fn a_row_that_moved_over_the_same_nodes_is_found_where_it_is() {
    let mut refs = Refs::default();
    let given = read(
        &mut refs,
        &list(&[
            (&["Ana"], "Edit", 10),
            (&["Bea"], "Edit", 11),
            (&["Cid"], "Edit", 12),
        ]),
    );
    let without_ana = list(&[(&["Bea"], "Edit", 10), (&["Cid"], "Edit", 11)]);
    let without_bea = list(&[(&["Ana"], "Edit", 10), (&["Cid"], "Edit", 11)]);

    assert_eq!(find(&refs, &given[0], &without_ana), None);
    assert_eq!(find(&refs, &given[1], &without_ana), Some(10));
    assert_eq!(find(&refs, &given[2], &without_ana), Some(11));
    assert_eq!(find(&refs, &given[0], &without_bea), Some(10));
    assert_eq!(find(&refs, &given[1], &without_bea), None);
}

#[test]
fn a_row_that_is_gone_never_passes_for_the_one_left() {
    let mut refs = Refs::default();
    let given = read(
        &mut refs,
        &list(&[(&["Alpha"], "Delete", 10), (&["Beta"], "Delete", 11)]),
    );
    assert_eq!(
        find(&refs, &given[0], &list(&[(&["Beta"], "Delete", 11)])),
        None
    );

    let twins = read(
        &mut refs,
        &list(&[(&["Untitled"], "Delete", 20), (&["Untitled"], "Delete", 21)]),
    );
    let left = list(&[(&["Untitled"], "Delete", 21)]);
    assert_eq!(find(&refs, &twins[0], &left), None);
    assert_eq!(find(&refs, &twins[1], &left), Some(21));
}

#[test]
fn a_replaced_element_is_found_only_where_the_same_is_said_around_it() {
    let mut refs = Refs::default();
    let given = read(&mut refs, &list(&[(&["Archive project?"], "Confirm", 10)]));

    assert_eq!(
        find(
            &refs,
            &given[0],
            &list(&[(&["Archive project?"], "Confirm", 30)])
        ),
        Some(30)
    );
    assert_eq!(
        find(
            &refs,
            &given[0],
            &list(&[(&["Delete account?"], "Confirm", 30)])
        ),
        None
    );
}

#[test]
fn an_element_that_changes_in_place_stays_itself() {
    let mut refs = Refs::default();
    let given = read(
        &mut refs,
        &list(&[
            (&["1s ago", "Ana"], "Like 12", 10),
            (&["2s ago", "Bea"], "Like 3", 11),
            (&["3s ago", "Cid"], "Like 4", 12),
        ]),
    );
    let liked = list(&[
        (&["4s ago", "Ana"], "Like 13", 10),
        (&["2s ago", "Bea"], "Like 3", 11),
        (&["3s ago", "Cid"], "Like 4", 12),
    ]);
    assert_eq!(find(&refs, &given[0], &liked), Some(10));

    let alone = read(&mut refs, &list(&[(&[], "Resend in 30s", 20)]));
    let ticked = list(&[(&[], "Resend in 29s", 20)]);
    assert_eq!(find(&refs, &alone[0], &ticked), Some(20));
    assert_eq!(read(&mut refs, &ticked), alone);
}

#[test]
fn rows_numbered_anew_stay_themselves() {
    let mut refs = Refs::default();
    let given = read(
        &mut refs,
        &list(&[
            (&["1.", "Ana"], "Edit", 10),
            (&["2.", "Bea"], "Edit", 11),
            (&["3.", "Cid"], "Edit", 12),
        ]),
    );
    let now = list(&[(&["1.", "Bea"], "Edit", 11), (&["2.", "Cid"], "Edit", 12)]);

    assert_eq!(find(&refs, &given[0], &now), None);
    assert_eq!(find(&refs, &given[1], &now), Some(11));
    assert_eq!(find(&refs, &given[2], &now), Some(12));
}

#[test]
fn rows_scrolled_over_the_same_nodes_aren_t_the_ones_read() {
    let mut refs = Refs::default();
    let rows = |from: i64| {
        let texts: Vec<String> = (from..from + 4).map(|row| format!("Order {row}")).collect();
        let cells: Vec<[&str; 2]> = texts
            .iter()
            .map(|text| [text.as_str(), "Open it"])
            .collect();
        let rows: Vec<(&[&str], &str, i64)> = cells
            .iter()
            .enumerate()
            .map(|(at, cells)| (cells.as_slice(), "Edit", 10 + at as i64))
            .collect();
        list(&rows)
    };
    let given = read(&mut refs, &rows(1));

    assert_eq!(find(&refs, &given[0], &rows(50)), None);
    assert_eq!(find(&refs, &given[3], &rows(3)), Some(11));
}

#[test]
fn a_row_that_took_another_s_place_alone_isn_t_the_one_read() {
    let mut refs = Refs::default();
    let given = read(
        &mut refs,
        &list(&[(&["Alpha"], "Delete", 10), (&["Beta"], "Delete", 11)]),
    );
    let now = list(&[(&["Gamma"], "Delete", 10), (&["Beta"], "Delete", 11)]);

    assert_eq!(find(&refs, &given[0], &now), None);
    assert_eq!(find(&refs, &given[1], &now), Some(11));
}

#[test]
fn an_element_drawn_again_keeps_its_ref() {
    let mut refs = Refs::default();
    let given = read(
        &mut refs,
        &list(&[(&["Ana"], "Edit", 10), (&["Bea"], "Edit", 11)]),
    );
    let drawn = list(&[(&["Bea"], "Edit", 21), (&["Ana"], "Edit", 20)]);
    assert_eq!(
        read(&mut refs, &drawn),
        [given[1].clone(), given[0].clone()]
    );

    let swapped = list(&[(&["Ana"], "Edit", 21), (&["Bea"], "Edit", 20)]);
    assert_eq!(read(&mut refs, &swapped), given);
}

#[test]
fn a_node_that_became_another_row_gets_that_row_s_ref() {
    let mut refs = Refs::default();
    let before = read(
        &mut refs,
        &list(&[(&["Ana"], "Edit", 10), (&["Bea"], "Edit", 11)]),
    );
    let now = list(&[(&["Bea"], "Edit", 10)]);

    let after = read(&mut refs, &now);

    assert_eq!(after[0], before[1]);
    assert_eq!(find(&refs, &before[0], &now), None);
    assert_eq!(find(&refs, &after[0], &now), Some(10));
}

#[test]
fn another_document_has_none_of_the_refs_even_with_the_same_node_ids() {
    let mut refs = Refs::default();
    let rows = list(&[(&["Ana"], "Edit", 10)]);
    let given = read(&mut refs, &rows);
    let elsewhere = Place {
        document: "2".into(),
        url: "/".into(),
    };

    let lost = refs.find(&given[0], &Tree::new(&rows), &elsewhere);
    refs.begin(&Tree::new(&rows), elsewhere.clone());
    let cleared = refs.find(&given[0], &Tree::new(&rows), &elsewhere);

    assert!(matches!(lost, Err(Lost::Changed(_))));
    assert!(cleared.is_err());
}

#[test]
fn old_refs_go_once_there_are_too_many() {
    let mut refs = Refs::default();
    let names: Vec<String> = (0..=MAX_KEPT).map(|n| format!("b{n}")).collect();
    let many: Vec<(&[&str], &str, i64)> = names
        .iter()
        .enumerate()
        .map(|(n, name)| (&["row"][..], name.as_str(), n as i64 + 10))
        .collect();
    let old = read(&mut refs, &list(&many));
    let now = list(&[(&["Ana"], "Edit", 5)]);
    let latest = read(&mut refs, &now);

    assert!(refs.targets.contains_key(&old[0]));
    refs.trim();

    assert!(!refs.targets.contains_key(&old[0]));
    assert_eq!(find(&refs, &latest[0], &now), Some(5));
}
