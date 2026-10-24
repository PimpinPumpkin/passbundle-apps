//! Where items live, as the web vault's menu shows it (passbundle-clients,
//! apps/web/src/app/keykeeper/vault-menu): Personal with the person's own
//! folders, then each organization with its shared folders (upstream's
//! collections). A name with "/" in it nests under the folder named for
//! what comes before, as upstream's apps nest them (`ServiceUtils.nestedTraverse`):
//! "Work/Clients" sits inside "Work" when there's a "Work", and is a folder
//! of its own at the top otherwise.

/// One folder or shared folder, in the order the menu lists them (each
/// before what's inside it), with how deep it is and what it's called there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nested {
    pub id: String,
    /// The whole name, as stored.
    pub name: String,
    /// What it's called where it sits: the last part of a nested name.
    pub label: String,
    /// 0 at the top; 1 inside another; and so on.
    pub depth: usize,
    pub parent: Option<String>,
}

struct Node {
    id: String,
    name: String,
    label: String,
    parent: Option<String>,
    children: Vec<Node>,
}

/// Puts one named folder into the tree, as upstream's apps do.
fn place(
    tree: &mut Vec<Node>,
    at: usize,
    parts: &[String],
    id: &str,
    name: &str,
    parent: Option<&str>,
) {
    if parts.len() <= at {
        return;
    }
    let end = at == parts.len() - 1;
    let part = &parts[at];
    if let Some(i) = tree.iter().position(|n| &n.label == part) {
        if end && tree[i].id != id {
            // Another folder of the same name, beside it.
            tree.push(Node {
                id: id.to_owned(),
                name: name.to_owned(),
                label: part.clone(),
                parent: parent.map(str::to_owned),
                children: Vec::new(),
            });
            return;
        }
        let inside = tree[i].id.clone();
        place(
            &mut tree[i].children,
            at + 1,
            parts,
            id,
            name,
            Some(&inside),
        );
        return;
    }
    if end {
        tree.push(Node {
            id: id.to_owned(),
            name: name.to_owned(),
            label: part.clone(),
            parent: parent.map(str::to_owned),
            children: Vec::new(),
        });
        return;
    }
    // No folder for this part: it and the next are one name here.
    let mut joined = vec![format!("{part}/{}", parts[at + 1])];
    joined.extend_from_slice(&parts[at + 2..]);
    place(tree, 0, &joined, id, name, parent);
}

fn flatten(tree: Vec<Node>, depth: usize, out: &mut Vec<Nested>) {
    for node in tree {
        out.push(Nested {
            id: node.id,
            name: node.name,
            label: node.label,
            depth,
            parent: node.parent,
        });
        flatten(node.children, depth + 1, out);
    }
}

/// Folders (id, name) nested by their names, sorted by name, ignoring case.
pub fn nest(folders: &[(String, String)]) -> Vec<Nested> {
    let mut sorted: Vec<&(String, String)> = folders.iter().collect();
    sorted.sort_by(|a, b| {
        a.1.to_lowercase()
            .cmp(&b.1.to_lowercase())
            .then_with(|| a.0.cmp(&b.0))
    });
    let mut tree = Vec::new();
    for (id, name) in sorted {
        let parts: Vec<String> = name.split('/').map(str::to_owned).collect();
        place(&mut tree, 0, &parts, id, name, None);
    }
    let mut out = Vec::new();
    flatten(tree, 0, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(names: &[&str]) -> Vec<(String, String)> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| (format!("id{i}"), (*n).to_owned()))
            .collect()
    }

    fn shown(nested: &[Nested]) -> Vec<String> {
        nested
            .iter()
            .map(|n| format!("{}{}", "  ".repeat(n.depth), n.label))
            .collect()
    }

    #[test]
    fn folders_nest_under_the_folder_their_name_starts_with() {
        let nested = nest(&named(&[
            "Work/Clients",
            "Banking",
            "work",
            "Work",
            "Work/Clients/Acme",
        ]));
        assert_eq!(
            shown(&nested),
            ["Banking", "work", "Work", "  Clients", "    Acme"],
        );
        let clients = nested.iter().find(|n| n.label == "Clients").unwrap();
        assert_eq!(clients.name, "Work/Clients");
        assert_eq!(clients.parent.as_deref(), Some("id3"));
    }

    #[test]
    fn a_name_whose_first_part_is_no_folder_stays_whole() {
        let nested = nest(&named(&["Home/Bills", "Home/Bills/Water", "Admins"]));
        assert_eq!(shown(&nested), ["Admins", "Home/Bills", "  Water"]);
    }
}
