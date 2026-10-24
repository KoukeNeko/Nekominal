//! What a screen reader is told about a window: the tabs, the text of the terminal that has the keyboard, and the
//! pop-up that is open. The tree is built from the state of the window only when a client has asked for it (see
//! `App::access_update`), so a window nobody listens to pays nothing.

use accesskit::{ActionHandler, ActionRequest, ActivationHandler, Node, NodeId, Role, Tree, TreeId, TreeUpdate};

const WINDOW: NodeId = NodeId(0);
const TABS: NodeId = NodeId(1);
const TERMINAL: NodeId = NodeId(2);
const POPUP: NodeId = NodeId(3);
const FIRST_TAB: u64 = 100;
const FIRST_ROW: u64 = 1000;

/// What the tree is built from.
pub struct View {
    pub window: String,
    pub tabs_label: String,
    /// The title of each tab, whether it is the selected one, and where its chip is drawn now (left, top, right, bottom,
    /// in pixels from the top left of the window).
    pub tabs: Vec<(String, bool, [f64; 4])>,
    pub terminal_label: String,
    /// The text on the screen of the terminal that has the keyboard.
    pub terminal_text: String,
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
    let mut children = vec![TABS, TERMINAL];

    let mut tabs = Node::new(Role::TabList);
    tabs.set_label(view.tabs_label.as_str());
    let mut tab_ids = Vec::new();
    for (i, (title, selected, [x0, y0, x1, y1])) in view.tabs.iter().enumerate() {
        let id = NodeId(FIRST_TAB + i as u64);
        let mut tab = Node::new(Role::Tab);
        tab.set_label(title.as_str());
        tab.set_selected(*selected);
        tab.set_bounds(accesskit::Rect { x0: *x0, y0: *y0, x1: *x1, y1: *y1 });
        nodes.push((id, tab));
        tab_ids.push(id);
    }
    tabs.set_children(tab_ids);
    nodes.push((TABS, tabs));

    let mut terminal = Node::new(Role::Terminal);
    terminal.set_label(view.terminal_label.as_str());
    terminal.set_value(view.terminal_text.as_str());
    nodes.push((TERMINAL, terminal));

    let mut focus = TERMINAL;
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
        focus = popup.focused.map_or(POPUP, |i| NodeId(FIRST_ROW + i as u64));
    }
    window.set_children(children);
    nodes.push((WINDOW, window));

    TreeUpdate { nodes, tree: Some(Tree::new(WINDOW)), tree_id: TreeId::ROOT, focus }
}

/// What the tree of the window of the settings is built from.
pub struct PageView {
    pub window: String,
    pub sections_label: String,
    /// The name of each section and whether it is the one shown (none is while a search shows rows of every section).
    pub sections: Vec<(String, bool)>,
    pub search_label: String,
    pub query: String,
    /// The label and the value of each row on show.
    pub rows: Vec<(String, String)>,
    /// The row that has the keyboard.
    pub focused: Option<usize>,
}

/// The settings: the sections as tabs, the search line, and the rows as a list. The keyboard is on the row that has it,
/// on the search line otherwise.
pub fn page_tree(view: &PageView) -> TreeUpdate {
    const SEARCH: NodeId = NodeId(4);
    let mut nodes = Vec::new();
    let mut window = Node::new(Role::Window);
    window.set_label(view.window.as_str());

    let mut tabs = Node::new(Role::TabList);
    tabs.set_label(view.sections_label.as_str());
    let mut tab_ids = Vec::new();
    for (i, (name, selected)) in view.sections.iter().enumerate() {
        let id = NodeId(FIRST_TAB + i as u64);
        let mut tab = Node::new(Role::Tab);
        tab.set_label(name.as_str());
        tab.set_selected(*selected);
        nodes.push((id, tab));
        tab_ids.push(id);
    }
    tabs.set_children(tab_ids);
    nodes.push((TABS, tabs));

    let mut search = Node::new(Role::SearchInput);
    search.set_label(view.search_label.as_str());
    search.set_value(view.query.as_str());
    nodes.push((SEARCH, search));

    let mut list = Node::new(Role::ListBox);
    let mut row_ids = Vec::new();
    for (i, (label, value)) in view.rows.iter().enumerate() {
        let id = NodeId(FIRST_ROW + i as u64);
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
    nodes.push((POPUP, list));

    window.set_children(vec![TABS, SEARCH, POPUP]);
    nodes.push((WINDOW, window));
    let focus = view.focused.map_or(SEARCH, |i| NodeId(FIRST_ROW + i as u64));
    TreeUpdate { nodes, tree: Some(Tree::new(WINDOW)), tree_id: TreeId::ROOT, focus }
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
            tabs: vec![("pwsh".into(), false, [10.0, 8.0, 110.0, 40.0]), ("claude".into(), true, [114.0, 8.0, 214.0, 40.0])],
            terminal_label: "Terminal".into(),
            terminal_text: "PS C:\\> echo hi\nhi".into(),
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
        let bounds = node(&update, tabs[1]).bounds().expect("a tab has bounds");
        assert_eq!((bounds.x0, bounds.x1), (114.0, 214.0));
    }

    #[test]
    fn the_settings_are_tabs_a_search_line_and_a_list_of_rows_with_their_values() {
        let view = PageView {
            window: "Settings".into(),
            sections_label: "Sections".into(),
            sections: vec![("Appearance".into(), true), ("Terminal".into(), false)],
            search_label: "Search settings".into(),
            query: String::new(),
            rows: vec![("Background opacity".into(), "72%".into()), ("Font size".into(), "14".into())],
            focused: None,
        };
        let update = page_tree(&view);
        assert_eq!(update.focus, NodeId(4), "the search line has the keyboard when no row has it");
        assert_eq!(node(&update, WINDOW).children(), &[TABS, NodeId(4), POPUP]);
        assert_eq!(node(&update, NodeId(FIRST_TAB)).is_selected(), Some(true));
        assert_eq!(node(&update, NodeId(FIRST_ROW)).label(), Some("Background opacity"));
        assert_eq!(node(&update, NodeId(FIRST_ROW)).description(), Some("72%"));
        let update = page_tree(&PageView { focused: Some(1), query: "fo".into(), ..view });
        assert_eq!(update.focus, NodeId(FIRST_ROW + 1));
        assert_eq!(node(&update, NodeId(4)).value(), Some("fo"));
        assert_eq!(node(&update, NodeId(FIRST_ROW + 1)).is_selected(), Some(true));
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
