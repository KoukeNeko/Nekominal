//! What a screen reader is told about a window: the tabs, the text of the terminal that has the keyboard, and the
//! pop-up that is open. The tree is built from the state of the window only when a client has asked for it (see
//! `App::access_update`), so a window nobody listens to pays nothing.

use accesskit::{ActionHandler, ActionRequest, ActivationHandler, Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};

const WINDOW: NodeId = NodeId(0);
const TABS: NodeId = NodeId(1);
const TERMINAL: NodeId = NodeId(2);
const POPUP: NodeId = NodeId(3);
/// The settings page, in the place of the terminal while the settings tab is in view: its sections, its search line and
/// its rows, numbered apart from the tabs and the rows of a pop-up, which can be there with them.
const PAGE: NodeId = NodeId(5);
const SECTIONS: NodeId = NodeId(6);
const SEARCH: NodeId = NodeId(4);
const PAGE_ROWS: NodeId = NodeId(7);
const FIRST_TAB: u64 = 100;
const FIRST_SECTION: u64 = 500;
const FIRST_ROW: u64 = 1000;
const FIRST_PAGE_ROW: u64 = 100_000;

/// What the tree is built from.
pub struct View {
    pub window: String,
    pub tabs_label: String,
    /// The title of each tab, whether it is the selected one, where its chip is drawn now (left, top, right, bottom, in
    /// pixels from the top left of the window), and its badges in words (empty without any).
    pub tabs: Vec<(String, bool, [f64; 4], String)>,
    pub terminal_label: String,
    /// The text on the screen of the terminal that has the keyboard.
    pub terminal_text: String,
    /// The settings page, shown instead of the terminal.
    pub page: Option<PageView>,
    pub popup: Option<PopupView>,
}

pub struct PopupView {
    pub kind: PopupKind,
    /// The question of a confirmation, the query of a palette or of the settings, empty for a menu.
    pub heading: String,
    /// The label and the hint (a shortcut, a value) of each choice; a separator is left out by the caller.
    pub rows: Vec<(String, String)>,
    /// The choice that has the keyboard.
    pub focused: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupKind {
    Menu,
    List,
    Dialog,
}

pub fn tree(view: &View) -> TreeUpdate {
    let mut nodes = Vec::new();
    let mut window = Node::new(Role::Window);
    window.set_label(view.window.as_str());
    let mut children = vec![TABS];

    let mut tabs = Node::new(Role::TabList);
    tabs.set_label(view.tabs_label.as_str());
    let mut tab_ids = Vec::new();
    for (i, (title, selected, [x0, y0, x1, y1], badges)) in view.tabs.iter().enumerate() {
        let id = NodeId(FIRST_TAB + i as u64);
        let mut tab = Node::new(Role::Tab);
        tab.set_label(title.as_str());
        if !badges.is_empty() {
            tab.set_description(badges.as_str());
        }
        tab.set_selected(*selected);
        tab.set_bounds(accesskit::Rect { x0: *x0, y0: *y0, x1: *x1, y1: *y1 });
        nodes.push((id, tab));
        tab_ids.push(id);
    }
    tabs.set_children(tab_ids);
    nodes.push((TABS, tabs));

    let mut focus = match &view.page {
        Some(page) => {
            children.push(PAGE);
            page_nodes(page, &mut nodes)
        }
        None => {
            let mut terminal = Node::new(Role::Terminal);
            terminal.set_label(view.terminal_label.as_str());
            terminal.set_value(view.terminal_text.as_str());
            nodes.push((TERMINAL, terminal));
            children.push(TERMINAL);
            TERMINAL
        }
    };
    if let Some(popup) = &view.popup {
        let (role, row_role) = match popup.kind {
            PopupKind::Menu => (Role::Menu, Role::MenuItem),
            PopupKind::List => (Role::ListBox, Role::ListBoxOption),
            PopupKind::Dialog => (Role::AlertDialog, Role::Button),
        };
        let mut node = Node::new(role);
        node.set_label(popup.heading.as_str());
        let mut row_ids = Vec::new();
        for (i, (label, hint)) in popup.rows.iter().enumerate() {
            let id = NodeId(FIRST_ROW + i as u64);
            let mut row = Node::new(row_role);
            row.set_label(label.as_str());
            if !hint.is_empty() {
                row.set_description(hint.as_str());
            }
            row.set_selected(popup.focused == Some(i));
            nodes.push((id, row));
            row_ids.push(id);
        }
        node.set_children(row_ids);
        nodes.push((POPUP, node));
        children.push(POPUP);
        // A focus past the rows would name a node that is not in the tree, which accesskit refuses with a panic.
        focus = popup.focused.filter(|&i| i < popup.rows.len()).map_or(POPUP, |i| NodeId(FIRST_ROW + i as u64));
    }
    window.set_children(children);
    nodes.push((WINDOW, window));

    TreeUpdate { nodes, tree: Some(TreeInfo::new(WINDOW)), tree_id: TreeId::ROOT, focus }
}

/// What the settings page in the tree is built from.
pub struct PageView {
    pub sections_label: String,
    /// The name of each section, whether it is the one shown (none is while a search shows rows of every section) and
    /// where its tab is drawn (left, top, right, bottom, in pixels from the top left of the window).
    pub sections: Vec<(String, bool, [f64; 4])>,
    pub search_label: String,
    pub query: String,
    /// The label and the value of each row on show.
    pub rows: Vec<(String, String)>,
    /// The row that has the keyboard.
    pub focused: Option<usize>,
}

/// The settings, added to `nodes` under `PAGE`: the sections as tabs, the search line, and the rows as a list. What has the
/// keyboard is returned: the row that has it, the search line otherwise.
fn page_nodes(view: &PageView, nodes: &mut Vec<(NodeId, Node)>) -> NodeId {
    let mut page = Node::new(Role::Group);
    page.set_label(view.sections_label.as_str());

    let mut tabs = Node::new(Role::TabList);
    tabs.set_label(view.sections_label.as_str());
    let mut tab_ids = Vec::new();
    for (i, (name, selected, [x0, y0, x1, y1])) in view.sections.iter().enumerate() {
        let id = NodeId(FIRST_SECTION + i as u64);
        let mut tab = Node::new(Role::Tab);
        tab.set_label(name.as_str());
        tab.set_selected(*selected);
        tab.set_bounds(accesskit::Rect { x0: *x0, y0: *y0, x1: *x1, y1: *y1 });
        nodes.push((id, tab));
        tab_ids.push(id);
    }
    tabs.set_children(tab_ids);
    nodes.push((SECTIONS, tabs));

    let mut search = Node::new(Role::SearchInput);
    search.set_label(view.search_label.as_str());
    search.set_value(view.query.as_str());
    nodes.push((SEARCH, search));

    let mut list = Node::new(Role::ListBox);
    let mut row_ids = Vec::new();
    for (i, (label, value)) in view.rows.iter().enumerate() {
        let id = NodeId(FIRST_PAGE_ROW + i as u64);
        let mut row = Node::new(Role::ListBoxOption);
        row.set_label(label.as_str());
        if !value.is_empty() {
            row.set_description(value.as_str());
        }
        row.set_selected(view.focused == Some(i));
        nodes.push((id, row));
        row_ids.push(id);
    }
    list.set_children(row_ids);
    nodes.push((PAGE_ROWS, list));

    page.set_children(vec![SECTIONS, SEARCH, PAGE_ROWS]);
    nodes.push((PAGE, page));
    view.focused.filter(|&i| i < view.rows.len()).map_or(SEARCH, |i| NodeId(FIRST_PAGE_ROW + i as u64))
}

/// The reader may ask for actions (focus, click); the window answers keys and the pointer only.
pub struct NoActions;

impl ActionHandler for NoActions {
    fn do_action(&mut self, _request: ActionRequest) {}
}

/// Hands the adapter the tree that was built for the request.
pub struct Initial(pub Option<TreeUpdate>);

impl ActivationHandler for Initial {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        self.0.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(popup: Option<PopupView>) -> View {
        View {
            window: "Nekominal".into(),
            tabs_label: "Tabs".into(),
            tabs: vec![("pwsh".into(), false, [10.0, 8.0, 110.0, 40.0], String::new()), ("claude".into(), true, [114.0, 8.0, 214.0, 40.0], "#4 open · :3000".into())],
            terminal_label: "Terminal".into(),
            terminal_text: "PS C:\\> echo hi\nhi".into(),
            page: None,
            popup,
        }
    }

    fn node<'a>(update: &'a TreeUpdate, id: NodeId) -> &'a Node {
        &update.nodes.iter().find(|(node_id, _)| *node_id == id).expect("node is in the update").1
    }

    #[test]
    fn the_tabs_and_the_terminal_text_are_in_the_tree_and_the_terminal_has_the_keyboard() {
        let update = tree(&view(None));
        assert_eq!(update.focus, TERMINAL);
        assert_eq!(node(&update, TERMINAL).value(), Some("PS C:\\> echo hi\nhi"));
        assert_eq!(node(&update, WINDOW).children(), &[TABS, TERMINAL]);
        let tabs = node(&update, TABS).children().to_vec();
        assert_eq!(tabs.len(), 2);
        assert_eq!(node(&update, tabs[0]).is_selected(), Some(false));
        assert_eq!(node(&update, tabs[1]).is_selected(), Some(true));
        assert_eq!(node(&update, tabs[1]).label(), Some("claude"));
        assert_eq!(node(&update, tabs[1]).description(), Some("#4 open · :3000"), "the badges of a tab");
        assert_eq!(node(&update, tabs[0]).description(), None);
        let bounds = node(&update, tabs[1]).bounds().expect("a tab has bounds");
        assert_eq!((bounds.x0, bounds.x1), (114.0, 214.0));
    }

    fn page(focused: Option<usize>, query: &str, rows: Vec<(String, String)>) -> PageView {
        PageView {
            sections_label: "Sections".into(),
            sections: vec![("Appearance".into(), true, [14.0, 8.0, 100.0, 44.0]), ("Terminal".into(), false, [100.0, 8.0, 180.0, 44.0])],
            search_label: "Search settings".into(),
            query: query.into(),
            rows,
            focused,
        }
    }

    #[test]
    fn the_settings_tab_has_the_page_in_the_place_of_the_terminal_its_sections_a_search_line_and_its_rows() {
        let rows = vec![("Background opacity".to_owned(), "72%".to_owned()), ("Font size".to_owned(), "14".to_owned())];
        let update = tree(&View { page: Some(page(None, "", rows.clone())), ..view(None) });
        assert_eq!(node(&update, WINDOW).children(), &[TABS, PAGE], "the tabs of the window stay, the terminal is not there");
        assert!(update.nodes.iter().all(|(id, _)| *id != TERMINAL));
        assert_eq!(update.focus, SEARCH, "the search line has the keyboard when no row has it");
        assert_eq!(node(&update, PAGE).children(), &[SECTIONS, SEARCH, PAGE_ROWS]);
        assert_eq!(node(&update, TABS).children().len(), 2, "the sections are not taken for tabs of the window");
        assert_eq!(node(&update, NodeId(FIRST_SECTION)).is_selected(), Some(true));
        let bounds = node(&update, NodeId(FIRST_SECTION + 1)).bounds().expect("a section has bounds");
        assert_eq!((bounds.x0, bounds.x1), (100.0, 180.0));
        assert_eq!(node(&update, NodeId(FIRST_PAGE_ROW)).label(), Some("Background opacity"));
        assert_eq!(node(&update, NodeId(FIRST_PAGE_ROW)).description(), Some("72%"));
        let update = tree(&View { page: Some(page(Some(1), "fo", rows.clone())), ..view(None) });
        assert_eq!(update.focus, NodeId(FIRST_PAGE_ROW + 1));
        assert_eq!(node(&update, SEARCH).value(), Some("fo"));
        assert_eq!(node(&update, NodeId(FIRST_PAGE_ROW + 1)).is_selected(), Some(true));
        // A menu opened over the page takes the keyboard, and its rows are told apart from those of the page.
        let menu = PopupView { kind: PopupKind::Menu, heading: String::new(), rows: rows.clone(), focused: Some(0) };
        let update = tree(&View { page: Some(page(Some(1), "", rows)), ..view(Some(menu)) });
        assert_eq!(update.focus, NodeId(FIRST_ROW));
        let mut ids: Vec<NodeId> = update.nodes.iter().map(|(id, _)| *id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), update.nodes.len(), "every node has a number of its own");
    }

    /// accesskit panics on a focus that names no node of the tree, and the panic ends the process.
    #[test]
    fn the_focus_always_names_a_node_of_the_tree_even_with_no_rows() {
        let update = tree(&View { page: Some(PageView { sections: Vec::new(), ..page(Some(0), "zzzz", Vec::new()) }), ..view(None) });
        assert!(update.nodes.iter().any(|(id, _)| *id == update.focus), "a search that found nothing, and Down pressed");
        let update = tree(&view(Some(PopupView { kind: PopupKind::Menu, heading: String::new(), rows: Vec::new(), focused: Some(3) })));
        assert!(update.nodes.iter().any(|(id, _)| *id == update.focus), "a menu row that went away");
    }

    #[test]
    fn an_open_menu_takes_the_keyboard_to_the_row_that_is_chosen_and_a_dialog_is_an_alert() {
        let rows = vec![("New tab".to_owned(), "Ctrl+Shift+T".to_owned()), ("Find".to_owned(), String::new())];
        let update = tree(&view(Some(PopupView { kind: PopupKind::Menu, heading: String::new(), rows: rows.clone(), focused: Some(1) })));
        assert_eq!(update.focus, NodeId(FIRST_ROW + 1));
        assert_eq!(node(&update, NodeId(FIRST_ROW)).description(), Some("Ctrl+Shift+T"));
        assert_eq!(node(&update, NodeId(FIRST_ROW + 1)).description(), None);
        assert_eq!(node(&update, POPUP).role(), Role::Menu);

        let update = tree(&view(Some(PopupView { kind: PopupKind::Dialog, heading: "Paste 5.9 KiB?".into(), rows, focused: None })));
        assert_eq!(update.focus, POPUP);
        assert_eq!(node(&update, POPUP).role(), Role::AlertDialog);
        assert_eq!(node(&update, POPUP).label(), Some("Paste 5.9 KiB?"));
    }
}
