mod worktree_node;

use std::collections::VecDeque;
use std::io::Write;

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout},
    prelude::{Buffer, Rect},
    style::{Color, Modifier, Style, palette::tailwind::SLATE},
    text::{Line, Text},
    widgets::{
        Block, HighlightSpacing, List, ListState, ScrollbarOrientation, ScrollbarState,
        StatefulWidget, Widget,
    },
};
use worktree_node::WorkTreeNode;

use crate::{
    app::{
        Action, Actions,
        action::{
            ConfirmAction, EditJobAction, JobAction, NavigationAction, PreviewNavigationAction,
            SearchAction, TreeSearchAction, WorkSpaceAction,
        },
        component::confirm_dialog::{
            error_confirm_dialog::ErrorConfirmDialog, text_confirm_dialog::TextConfirmDialog,
        },
        config::Config,
        math::Op,
    },
    container::node::{AddNodeKey, Index, IndexKind, Node, NodeMeta},
    error::MutationError,
};

use super::{
    confirm_dialog::{ConfirmDialog, boolean_confirm_dialog::BooleanConfirmDialog},
    loading::Loading,
    preview::{Preview, PreviewState, SearchState},
    scrollbar::scrollbar,
};

const BFS_NODE_LIMIT: usize = 1_000_000;

struct TreeSearchState {
    query: String,
    is_input_mode: bool,
    bfs_queue: VecDeque<Vec<String>>,
    matches: Vec<Vec<String>>,
    current_match: usize,
    exhausted: bool,
}

pub struct WorkSpace {
    config: Config,
    file_root: Node,
    work_tree_root: WorkTreeNode,
    is_edited: bool,

    dialogs: Vec<Box<dyn ConfirmDialog>>,
    preview: Option<Preview>,
    preview_pct: u16,
    loading: Option<Loading>,
    search_active: bool,
    search_has_results: bool,
    tree_search_active: bool,
    tree_search_has_results: bool,
    tree_search_state: Option<TreeSearchState>,
}

impl WorkSpace {
    pub fn new(file_root: Node, config: Config) -> Self {
        let work_tree_root =
            WorkTreeNode::new(String::from("root"), Some(file_root.as_index().meta));
        Self {
            config,
            file_root,
            work_tree_root,
            is_edited: false,
            dialogs: Vec::new(),
            preview: None,
            preview_pct: 65,
            loading: None,
            search_active: false,
            search_has_results: false,
            tree_search_active: false,
            tree_search_has_results: false,
            tree_search_state: None,
        }
    }

    pub fn handle_event(&self, actions: &mut Actions, event: Event) {
        if self.loading.is_some() {
            return;
        }

        if let Some(dialog) = self.dialogs.last() {
            dialog.handle_event(actions, event);
            return;
        }

        let Some(event) = event.as_key_press_event() else {
            return;
        };

        if self.search_active {
            match event.code {
                KeyCode::Char(c) => actions.push(SearchAction::Input(c).into()),
                KeyCode::Backspace => actions.push(SearchAction::Backspace.into()),
                KeyCode::Enter => actions.push(SearchAction::Confirm.into()),
                KeyCode::Esc => actions.push(SearchAction::Cancel.into()),
                _ => {}
            }
            return;
        }

        if self.tree_search_active {
            match event.code {
                KeyCode::Char(c) => actions.push(TreeSearchAction::Input(c).into()),
                KeyCode::Backspace => actions.push(TreeSearchAction::Backspace.into()),
                KeyCode::Enter => actions.push(TreeSearchAction::Confirm.into()),
                KeyCode::Esc => actions.push(TreeSearchAction::Cancel.into()),
                _ => {}
            }
            return;
        }

        if event.modifiers == KeyModifiers::CONTROL {
            match event.code {
                KeyCode::Char('u') => {
                    actions.push(NavigationAction::Up(10).into());
                }
                KeyCode::Char('d') => {
                    actions.push(NavigationAction::Down(10).into());
                }
                KeyCode::Char('U') => {
                    actions.push(PreviewNavigationAction::Up(5).into());
                }
                KeyCode::Char('D') => {
                    actions.push(PreviewNavigationAction::Down(5).into());
                }
                KeyCode::Left => {
                    actions.push(NavigationAction::PreviewWindowResize(Op::Add(1)).into());
                }
                KeyCode::Right => {
                    actions.push(NavigationAction::PreviewWindowResize(Op::Sub(1)).into());
                }
                _ => {}
            }
            return;
        }

        if self.tree_search_has_results {
            match event.code {
                KeyCode::Char('n') => {
                    actions.push(TreeSearchAction::Next.into());
                    return;
                }
                KeyCode::Char('p') => {
                    actions.push(TreeSearchAction::Previous.into());
                    return;
                }
                _ => {}
            }
        } else if self.search_has_results {
            match event.code {
                KeyCode::Char('n') => {
                    actions.push(SearchAction::Next.into());
                    return;
                }
                KeyCode::Char('p') => {
                    actions.push(SearchAction::Previous.into());
                    return;
                }
                _ => {}
            }
        }

        match event.code {
            KeyCode::Char('g') => {
                actions.push(NavigationAction::Top.into());
            }
            KeyCode::Char('G') => {
                actions.push(NavigationAction::Bottom.into());
            }
            KeyCode::Char('k') | KeyCode::Up => {
                actions.push(NavigationAction::Up(1).into());
            }
            KeyCode::Char('j') | KeyCode::Down => {
                actions.push(NavigationAction::Down(1).into());
            }
            KeyCode::Char('l') | KeyCode::Char(' ') | KeyCode::Tab => {
                actions.push(NavigationAction::Expand.into());
            }
            KeyCode::Enter => {
                actions.push(SearchAction::Next.into());
            }
            KeyCode::Char('/') => {
                actions.push(SearchAction::Start.into());
            }
            KeyCode::Char('?') => {
                actions.push(TreeSearchAction::Start.into());
            }
            KeyCode::Char('h') => {
                actions.push(NavigationAction::Close.into());
            }
            KeyCode::BackTab => {
                actions.push(NavigationAction::CloseOrCloseParent.into());
            }
            KeyCode::Char('p') => {
                actions.push(NavigationAction::TogglePreview.into());
            }
            KeyCode::Char('q') => {
                actions.push(Action::Exit(ConfirmAction::Request(())));
            }
            KeyCode::Char('e') => {
                actions.push(WorkSpaceAction::Edit.into());
            }
            KeyCode::Char('w') => {
                actions.push(WorkSpaceAction::Save(ConfirmAction::Request(())).into());
            }
            KeyCode::Char('H') => {
                actions.push(PreviewNavigationAction::Left.into());
            }
            KeyCode::Char('J') => {
                actions.push(PreviewNavigationAction::Down(1).into());
            }
            KeyCode::Char('K') => {
                actions.push(PreviewNavigationAction::Up(1).into());
            }
            KeyCode::Char('L') => {
                actions.push(PreviewNavigationAction::Right.into());
            }
            KeyCode::Char('r') => {
                actions.push(WorkSpaceAction::Rename(ConfirmAction::Request(())).into());
            }
            KeyCode::Char('d') => {
                actions.push(WorkSpaceAction::Delete(ConfirmAction::Request(())).into());
            }
            KeyCode::Char('a') => {
                actions.push(WorkSpaceAction::Add(ConfirmAction::Request(())).into());
            }
            _ => {}
        }
    }

    pub fn set_loading(&mut self, is_loading: bool) {
        if is_loading && self.loading.is_none() {
            self.loading = Some(Loading::default());
        } else if !is_loading {
            self.loading = None;
        }
    }

    pub fn maybe_exit(&mut self, confirm_action: ConfirmAction<()>) -> bool {
        match confirm_action {
            ConfirmAction::Request(()) => {
                if self.is_edited {
                    self.dialogs.push(Box::new(BooleanConfirmDialog::new(
                        Text::from(vec![Line::from("Discard unsaved changes?").centered()]),
                        Box::new(ConfirmAction::action_confirmer(Action::Exit)),
                    )));
                }

                !self.is_edited
            }
            ConfirmAction::Confirm(ok) => {
                self.dialogs.pop();
                ok
            }
        }
    }

    pub(crate) fn handle_action(
        &mut self,
        state: &mut WorkSpaceState,
        actions: &mut Actions,
        action: WorkSpaceAction,
    ) -> std::io::Result<()> {
        match action {
            WorkSpaceAction::Navigation(navigation_action) => {
                self.handle_navigation_action(state, navigation_action);
            }
            WorkSpaceAction::Edit => actions.push(JobAction::Edit(EditJobAction::Init).into()),
            WorkSpaceAction::EditError(confirm_action) => {
                if self.handle_edit_error_action(confirm_action) {
                    actions.push(JobAction::Edit(EditJobAction::Open).into());
                }
            }
            WorkSpaceAction::Rename(confirm_action) => {
                self.handle_rename(state, confirm_action)?;
            }
            WorkSpaceAction::Delete(confirm_action) => {
                self.handle_delete(state, confirm_action)?;
            }
            WorkSpaceAction::Add(confirm_action) => {
                self.handle_add(state, confirm_action)?;
            }
            WorkSpaceAction::Save(confirm_action) => {
                self.dialogs.pop();
                if let Some(action) = self.handle_save_action(confirm_action)? {
                    actions.push(action);
                }
            }
            WorkSpaceAction::SaveDone => self.handle_save_done(),
            WorkSpaceAction::Load { node, is_edit } => {
                self.replace_selected(state, node);
                self.is_edited |= is_edit;
            }
            WorkSpaceAction::ErrorConfirmed => {
                self.dialogs.pop();
            }
        }

        Ok(())
    }

    fn handle_navigation_action(
        &mut self,
        state: &mut WorkSpaceState,
        navigation_action: NavigationAction,
    ) {
        let prev_index = state.list_state.selected();
        match navigation_action {
            NavigationAction::Up(n) => {
                let index = state.list_state.selected().unwrap().saturating_sub(n);
                state.list_state.select(Some(index));
            }
            NavigationAction::Down(n) => {
                let index = state
                    .list_state
                    .selected()
                    .unwrap()
                    .saturating_add(n)
                    .min(self.work_tree_root.len().saturating_sub(1));
                state.list_state.select(Some(index));
            }
            NavigationAction::Top => {
                state.list_state.select_first();
            }
            NavigationAction::Bottom => {
                state.list_state.select(Some(self.work_tree_root.len() - 1));
            }
            NavigationAction::Expand => {
                if let Some(index) = state.list_state.selected() {
                    if self.expand(index) {
                        state.list_state.select_next();
                    }
                }
            }
            NavigationAction::Close => {
                if let Some(index) = state.list_state.selected() {
                    self.work_tree_root.close(index);
                }
            }
            NavigationAction::CloseOrCloseParent => {
                if let Some(index) = state.list_state.selected() {
                    if self.work_tree_root.is_expanded(index) {
                        self.work_tree_root.close(index);
                    } else if let Some(parent) = self.work_tree_root.parent_index(index) {
                        self.work_tree_root.close(parent);
                        state.list_state.select(Some(parent));
                    }
                }
            }
            NavigationAction::TogglePreview => {
                self.toggle_preview(state);
            }
            NavigationAction::PreviewNavigation(preview_navigation) => match preview_navigation {
                PreviewNavigationAction::Up(n) => state.preview_state.scroll_up(n),
                PreviewNavigationAction::Down(n) => state.preview_state.scroll_down(n),
                PreviewNavigationAction::Left => state.preview_state.scroll_left(),
                PreviewNavigationAction::Right => state.preview_state.scroll_right(),
            },
            NavigationAction::PreviewWindowResize(delta) => {
                self.preview_pct = delta.exec(self.preview_pct).clamp(20, 80)
            }
            NavigationAction::Search(action) => {
                self.handle_search_action(state, action);
            }
            NavigationAction::TreeSearch(action) => {
                self.handle_tree_search_action(state, action);
            }
        }

        if prev_index != state.list_state.selected() {
            self.set_preview_to_selected(state, false);
        }
    }

    fn expand(&mut self, index: usize) -> bool {
        if self.work_tree_root.is_expanded(index) {
            return false;
        }
        let selector = self.work_tree_root.selector(index);
        let node_index = self
            .file_root
            .subtree(&selector)
            .expect("broken selector")
            .as_index();
        let is_terminal = matches!(node_index.kind, IndexKind::Terminal);
        self.reindex(index, node_index, true);
        !is_terminal
    }

    pub fn selected_node(&self, worktree_state: &WorkSpaceState) -> Option<&Node> {
        let index = worktree_state.list_state.selected()?;
        let selector = self.work_tree_root.selector(index);
        Some(self.file_root.subtree(&selector).expect("broken selector"))
    }

    fn write_on_index(&self, mut writer: impl Write, index: usize) -> Result<(), std::io::Error> {
        let selector = self.work_tree_root.selector(index);
        let content = self
            .file_root
            .subtree(&selector)
            .expect("broken selector")
            .to_string_pretty()
            .expect("broken internal representation");
        writer.write_all(content.as_bytes())?;
        Ok(())
    }

    fn replace_selected(&mut self, worktree_state: &mut WorkSpaceState, new_node: Node) {
        let Some(index) = worktree_state.list_state.selected() else {
            return;
        };
        let selector = self.work_tree_root.selector(index);

        let node_index = new_node.as_index();
        self.file_root
            .replace(&selector, new_node)
            .expect("broken selector");
        self.reindex(index, node_index, false);
        self.tree_search_has_results = false;
        self.tree_search_state = None;
        self.set_preview_to_selected(worktree_state, false);
    }

    fn reindex(&mut self, index: usize, node_index: Index, force: bool) {
        self.work_tree_root.reindex(index, node_index, force);
    }

    fn toggle_preview(&mut self, state: &mut WorkSpaceState) {
        if self.preview.is_some() {
            self.preview = None;
            state.preview_state.clear_search();
            self.search_active = false;
            self.search_has_results = false;
            self.tree_search_has_results = false;
            self.tree_search_state = None;
            return;
        }

        self.set_preview_to_selected(state, true);
    }

    fn set_preview_to_selected(&mut self, state: &mut WorkSpaceState, force_show: bool) {
        if self.preview.is_none() && !force_show {
            return;
        }

        state.preview_state.clear_search();
        self.search_active = false;
        self.search_has_results = false;

        let Some(index) = state.list_state.selected() else {
            return;
        };
        let meta = self.meta_on_index(index);

        let mut buffer = Vec::new();
        if meta.n_bytes <= self.config.max_preview_size.as_u64() as usize {
            let _ = self.write_on_index(&mut buffer, index);
        }
        let preview = String::from_utf8(buffer).unwrap_or_default();
        self.preview = Some(Preview::new((!preview.is_empty()).then_some(preview)))
    }

    fn meta_on_index(&mut self, index: usize) -> NodeMeta {
        if let Some(meta) = self.work_tree_root.meta(index) {
            return meta;
        }

        let selector = self.work_tree_root.selector(index);
        let node_index = self
            .file_root
            .subtree(&selector)
            .expect("broken selector")
            .as_index();
        let meta = node_index.meta;
        self.reindex(index, node_index, false);
        meta
    }

    pub fn file_root(&self) -> &Node {
        &self.file_root
    }

    fn handle_search_action(&mut self, state: &mut WorkSpaceState, action: SearchAction) {
        match action {
            SearchAction::Start => {
                if self.preview.is_some() {
                    self.search_active = true;
                    self.tree_search_has_results = false;
                    self.tree_search_state = None;
                    state.preview_state.search = Some(SearchState {
                        query: String::new(),
                        is_input_mode: true,
                        matches: Vec::new(),
                        current_match: 0,
                    });
                }
            }
            SearchAction::Input(c) => {
                if let Some(search) = &mut state.preview_state.search {
                    search.query.push(c);
                }
            }
            SearchAction::Backspace => {
                if let Some(search) = &mut state.preview_state.search {
                    search.query.pop();
                }
            }
            SearchAction::Confirm => {
                self.search_active = false;
                if let Some(search) = &mut state.preview_state.search {
                    search.is_input_mode = false;
                }
                self.execute_search(state);
            }
            SearchAction::Cancel => {
                self.search_active = false;
                self.search_has_results = false;
                state.preview_state.clear_search();
            }
            SearchAction::Next => {
                let line = state.preview_state.search.as_mut().and_then(|search| {
                    if search.matches.is_empty() {
                        return None;
                    }
                    if search.current_match + 1 >= search.matches.len() {
                        return None;
                    }
                    search.current_match += 1;
                    let (line_idx, _) = search.matches[search.current_match];
                    Some(line_idx as u16)
                });
                if let Some(line) = line {
                    state.preview_state.set_y_offset(line);
                }
            }
            SearchAction::Previous => {
                let line = state.preview_state.search.as_mut().and_then(|search| {
                    if search.matches.is_empty() || search.current_match == 0 {
                        return None;
                    }
                    search.current_match -= 1;
                    let (line_idx, _) = search.matches[search.current_match];
                    Some(line_idx as u16)
                });
                if let Some(line) = line {
                    state.preview_state.set_y_offset(line);
                }
            }
        }
    }

    fn handle_tree_search_action(
        &mut self,
        state: &mut WorkSpaceState,
        action: TreeSearchAction,
    ) {
        match action {
            TreeSearchAction::Start => {
                self.tree_search_active = true;
                self.tree_search_has_results = false;
                self.tree_search_state = Some(TreeSearchState {
                    query: String::new(),
                    is_input_mode: true,
                    bfs_queue: VecDeque::new(),
                    matches: Vec::new(),
                    current_match: 0,
                    exhausted: false,
                });
                self.search_has_results = false;
                state.preview_state.clear_search();
                state.tree_search_match_index = None;
            }
            TreeSearchAction::Input(c) => {
                if let Some(ts) = &mut self.tree_search_state {
                    ts.query.push(c);
                }
            }
            TreeSearchAction::Backspace => {
                if let Some(ts) = &mut self.tree_search_state {
                    ts.query.pop();
                }
            }
            TreeSearchAction::Confirm => {
                self.tree_search_active = false;
                if let Some(ts) = &mut self.tree_search_state {
                    ts.is_input_mode = false;
                    if ts.query.is_empty() {
                        self.tree_search_state = None;
                        return;
                    }
                    ts.bfs_queue.push_back(vec![]);
                }
                self.bfs_find_next(state);
            }
            TreeSearchAction::Cancel => {
                self.tree_search_active = false;
                self.tree_search_has_results = false;
                self.tree_search_state = None;
                state.tree_search_match_index = None;
            }
            TreeSearchAction::Next => {
                let should_bfs = self.tree_search_state.as_ref().is_some_and(|ts| {
                    ts.current_match + 1 >= ts.matches.len()
                });
                if should_bfs {
                    self.bfs_find_next(state);
                } else if let Some(ts) = &mut self.tree_search_state {
                    ts.current_match += 1;
                    let path = ts.matches[ts.current_match].clone();
                    self.expand_to_path(state, &path);
                }
            }
            TreeSearchAction::Previous => {
                if let Some(ts) = &mut self.tree_search_state {
                    if ts.current_match > 0 {
                        ts.current_match -= 1;
                        let path = ts.matches[ts.current_match].clone();
                        self.expand_to_path(state, &path);
                    }
                }
            }
        }
    }

    fn bfs_find_next(&mut self, state: &mut WorkSpaceState) {
        let ts = match &mut self.tree_search_state {
            Some(ts) => ts,
            None => return,
        };

        let initial_match_count = ts.matches.len();
        let query_lower = ts.query.to_lowercase();
        let mut nodes_visited: usize = 0;

        while let Some(path) = ts.bfs_queue.pop_front() {
            nodes_visited += 1;
            if nodes_visited > BFS_NODE_LIMIT {
                ts.exhausted = true;
                ts.bfs_queue.clear();
                break;
            }

            let node = match self.file_root.subtree(&path) {
                Ok(node) => node,
                Err(_) => continue,
            };
            let index = node.as_index();
            match index.kind {
                IndexKind::Object(keys) => {
                    for key in keys {
                        let mut child_path = path.clone();
                        child_path.push(key.clone());
                        if key.to_lowercase().contains(&query_lower) {
                            ts.matches.push(child_path.clone());
                        }
                        ts.bfs_queue.push_back(child_path);
                    }
                }
                IndexKind::Array(n) => {
                    for i in 0..n {
                        let mut child_path = path.clone();
                        child_path.push(i.to_string());
                        ts.bfs_queue.push_back(child_path);
                    }
                }
                IndexKind::Terminal => {}
            }

            if ts.matches.len() > initial_match_count {
                break;
            }
        }

        if ts.matches.len() > initial_match_count {
            ts.current_match = initial_match_count;
            self.tree_search_has_results = true;
            let path = ts.matches[ts.current_match].clone();
            self.expand_to_path(state, &path);
        }
    }

    fn expand_to_path(&mut self, state: &mut WorkSpaceState, path: &[String]) {
        let mut current_idx = 0;

        // Expand root if not expanded
        if !self.work_tree_root.is_expanded(0) {
            self.expand(0);
        }

        for (i, segment) in path.iter().enumerate() {
            let children = match self.work_tree_root.direct_children(current_idx) {
                Some(c) => c,
                None => return,
            };

            let child_idx = children
                .iter()
                .find(|(_, name)| *name == segment.as_str())
                .map(|(idx, _)| *idx);

            let Some(child_idx) = child_idx else {
                return;
            };

            if i < path.len() - 1 {
                // Not the last segment — expand it
                if !self.work_tree_root.is_expanded(child_idx) {
                    self.expand(child_idx);
                }
            }

            current_idx = child_idx;
        }

        state.list_state.select(Some(current_idx));
        state.tree_search_match_index = Some(current_idx);
        self.set_preview_to_selected(state, false);
    }

    fn execute_search(&mut self, state: &mut WorkSpaceState) {
        let query = state
            .preview_state
            .search
            .as_ref()
            .map(|s| s.query.clone())
            .unwrap_or_default();
        if query.is_empty() {
            state.preview_state.clear_search();
            return;
        }

        let text = self
            .preview
            .as_ref()
            .and_then(|p| p.content_text())
            .unwrap_or_default();

        let mut matches = Vec::new();
        for (line_idx, line) in text.lines().enumerate() {
            let mut start = 0;
            while let Some(pos) = line[start..].find(&query) {
                matches.push((line_idx, start + pos));
                start += pos + query.len();
            }
        }

        let first_line = matches.first().map(|&(line_idx, _)| line_idx as u16);
        self.search_has_results = !matches.is_empty();
        if let Some(search) = &mut state.preview_state.search {
            search.matches = matches;
            search.current_match = 0;
        }
        if let Some(line) = first_line {
            state.preview_state.set_y_offset(line);
        }
    }
}

impl WorkSpace {
    fn handle_add(
        &mut self,
        state: &mut WorkSpaceState,
        confirm_action: ConfirmAction<(), Option<String>>,
    ) -> std::io::Result<()> {
        let Some(index) = self.index_for_mutation(state) else {
            return Ok(());
        };

        let new_key = match confirm_action {
            ConfirmAction::Request(_) => {
                let mut selector = self.work_tree_root.selector(index);
                selector.pop();
                let meta = self
                    .file_root
                    .subtree(&selector)
                    .expect("broken selector")
                    .as_index();

                if !matches!(meta.kind, IndexKind::Array(_)) {
                    self.dialogs.push(Box::new(
                        TextConfirmDialog::new(Box::new(ConfirmAction::action_confirmer(
                            WorkSpaceAction::Add,
                        )))
                        .title(Line::from("Append key")),
                    ));

                    return Ok(());
                }

                None
            }
            ConfirmAction::Confirm(new_key) => {
                self.dialogs.pop();
                let Some(new_key) = new_key else {
                    return Ok(());
                };
                Some(new_key)
            }
        };

        let add_node_key = match &new_key {
            Some(new_key) => AddNodeKey::Object(new_key.clone()),
            None => AddNodeKey::Array,
        };
        let mut selector = self.work_tree_root.selector(index);
        match self
            .file_root
            .append_after(&selector, add_node_key, Node::null())
        {
            Err(MutationError::DuplicateKey) => {
                self.dialogs.push(Box::new(
                    TextConfirmDialog::new(Box::new(ConfirmAction::action_confirmer(
                        WorkSpaceAction::Add,
                    )))
                    .title("Rename".into())
                    .content(new_key.unwrap_or_default()),
                ));
                self.dialogs
                    .push(Box::new(ErrorConfirmDialog::new("Duplicate key".into())));
                return Ok(());
            }
            Err(err) => {
                panic!("broken selector {err}")
            }
            Ok(_) => {}
        }
        selector.pop();
        let parent_metas = self.file_root.metas(&selector).expect("broken selector");
        self.work_tree_root
            .append_after(index, new_key, parent_metas);
        self.is_edited = true;
        state.list_state.select_next();
        self.set_preview_to_selected(state, false);

        Ok(())
    }

    fn handle_delete(
        &mut self,
        state: &mut WorkSpaceState,
        confirm_action: ConfirmAction<()>,
    ) -> std::io::Result<()> {
        let Some(index) = self.index_for_mutation(state) else {
            return Ok(());
        };

        match confirm_action {
            ConfirmAction::Request(_) => {
                self.dialogs.push(Box::new(BooleanConfirmDialog::new(
                    Text::from("Delete node?"),
                    Box::new(ConfirmAction::action_confirmer(WorkSpaceAction::Delete)),
                )));
            }
            ConfirmAction::Confirm(is_delete) => {
                self.dialogs.pop();
                if !is_delete {
                    return Ok(());
                }

                let mut selector = self.work_tree_root.selector(index);
                let _ = self.file_root.delete(&selector).expect("broken selector");
                selector.pop();
                let parent_metas = self.file_root.metas(&selector).expect("broken selector");
                self.work_tree_root.delete(index, parent_metas);

                if index >= self.work_tree_root.len() {
                    state.list_state.select_previous();
                }
                self.is_edited = true;
                self.set_preview_to_selected(state, false);
            }
        }

        Ok(())
    }

    fn handle_rename(
        &mut self,
        state: &WorkSpaceState,
        confirm_action: ConfirmAction<(), Option<String>>,
    ) -> std::io::Result<()> {
        let Some(index) = self.index_for_mutation(state) else {
            return Ok(());
        };
        match confirm_action {
            ConfirmAction::Request(_) => {
                let selector = self.work_tree_root.selector(index);
                let index = self
                    .file_root
                    .subtree(&selector[..selector.len() - 1])
                    .expect("broken selector")
                    .as_index();
                match index.kind {
                    IndexKind::Object(_) => {
                        self.dialogs.push(Box::new(
                            TextConfirmDialog::new(Box::new(ConfirmAction::action_confirmer(
                                WorkSpaceAction::Rename,
                            )))
                            .title("Rename".into())
                            .content(selector.last().expect("broken selector").to_string()),
                        ));
                    }
                    IndexKind::Array(_) | IndexKind::Terminal => {
                        self.dialogs.push(Box::new(ErrorConfirmDialog::new(
                            "Cannot rename list".into(),
                        )));
                    }
                }
            }
            ConfirmAction::Confirm(new_key) => {
                self.dialogs.pop();

                if let Some(new_key) = new_key {
                    let selector = self.work_tree_root.selector(index);
                    if selector
                        .last()
                        .is_some_and(|&old_key| old_key != new_key.as_str())
                    {
                        match self.file_root.rename(&selector, new_key.clone()) {
                            Ok(_) => {
                                self.work_tree_root.rename(index, new_key);
                                self.is_edited = true;
                            }
                            Err(MutationError::DuplicateKey) => {
                                self.dialogs.push(Box::new(
                                    TextConfirmDialog::new(Box::new(
                                        ConfirmAction::action_confirmer(WorkSpaceAction::Rename),
                                    ))
                                    .title("Rename".into())
                                    .content(new_key),
                                ));
                                self.dialogs.push(Box::new(ErrorConfirmDialog::new(
                                    "Duplicate key".into(),
                                )));
                            }
                            Err(err) => {
                                panic!("broken selector {err}")
                            }
                        };
                    }
                }
            }
        }

        Ok(())
    }

    fn index_for_mutation(&mut self, state: &WorkSpaceState) -> Option<usize> {
        let index = state.list_state.selected().unwrap_or_default();
        if index == 0 {
            self.dialogs.push(Box::new(
                ErrorConfirmDialog::new("Index cannot be 0".into())
                    .title(Line::from("Invalid selection")),
            ));
            return None;
        }

        Some(index)
    }
}

impl WorkSpace {
    fn handle_save_action(
        &mut self,
        confirm_action: ConfirmAction<()>,
    ) -> std::io::Result<Option<Action>> {
        match confirm_action {
            ConfirmAction::Request(()) => {
                self.dialogs.push(Box::new(BooleanConfirmDialog::new(
                    Text::from(Line::from("Write file?").centered()),
                    Box::new(ConfirmAction::action_confirmer(WorkSpaceAction::Save)),
                )));
                Ok(None)
            }
            ConfirmAction::Confirm(ok) => {
                if ok {
                    Ok(Some(JobAction::Save.into()))
                } else {
                    self.dialogs.pop();
                    Ok(None)
                }
            }
        }
    }

    fn handle_save_done(&mut self) {
        self.is_edited = false;
    }
}

impl WorkSpace {
    fn handle_edit_error_action(&mut self, confirm_action: ConfirmAction<String>) -> bool {
        match confirm_action {
            ConfirmAction::Request(message) => {
                let mut confirm_dialog = BooleanConfirmDialog::new(
                    Text::from(vec![
                        Line::from(message),
                        Line::from(""),
                        Line::from("Continue to edit?").centered(),
                    ]),
                    Box::new(ConfirmAction::action_confirmer(WorkSpaceAction::EditError)),
                );
                confirm_dialog.title(Some(Line::from("JSON Error").left_aligned()));
                self.dialogs.push(Box::new(confirm_dialog));
                false
            }
            ConfirmAction::Confirm(ok) => {
                self.dialogs.pop();
                ok
            }
        }
    }
}

#[derive(Debug)]
pub struct WorkSpaceState {
    list_state: ListState,
    preview_state: PreviewState,
    tree_view_offset: usize,
    tree_search_match_index: Option<usize>,
}

impl Default for WorkSpaceState {
    fn default() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            list_state,
            preview_state: PreviewState::default(),
            tree_view_offset: 0,
            tree_search_match_index: None,
        }
    }
}

impl StatefulWidget for &WorkSpace {
    type State = WorkSpaceState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        if let Some(preview) = &self.preview {
            let layout = Layout::horizontal([
                Constraint::Percentage(100 - self.preview_pct),
                Constraint::Fill(self.preview_pct),
            ]);
            let [tree_area, preview_area] = layout.areas(area);

            self.render_tree(tree_area, buf, state);
            preview.render(preview_area, buf, &mut state.preview_state);
        } else {
            self.render_tree(area, buf, state);
        }

        for dialog in &self.dialogs {
            dialog.render_ref(area, buf);
        }

        if let Some(loading) = &self.loading {
            loading.render(area, buf);
        }
    }
}

impl WorkSpace {
    fn render_tree(&self, area: Rect, buf: &mut Buffer, state: &mut WorkSpaceState) {
        let has_tree_search = self.tree_search_state.is_some();

        let (block_area, search_bar_y) = if has_tree_search && area.height > 2 {
            let mut block_area = area;
            block_area.height -= 1;
            (block_area, Some(area.y + area.height - 1))
        } else {
            (area, None)
        };

        let block = Block::bordered().title("Tree");
        let inner_area = block.inner(block_area);

        block.render(block_area, buf);

        let selected = state.list_state.selected();
        let entries = self.work_tree_root.as_windowed_tree_string(selected);
        let windowed_selected = selected
            .and_then(|sel| entries.iter().position(|e| e.real_index == Some(sel)));

        let list = List::new(entries.iter().map(|e| e.display.clone()))
            .highlight_style(Style::new().bg(SLATE.c800).add_modifier(Modifier::BOLD))
            .highlight_symbol("> ")
            .highlight_spacing(HighlightSpacing::Always)
            .scroll_padding(1);

        let mut temp_list_state = ListState::default();
        *temp_list_state.offset_mut() = state.tree_view_offset;
        temp_list_state.select(windowed_selected);
        StatefulWidget::render(&list, inner_area, buf, &mut temp_list_state);
        state.tree_view_offset = *temp_list_state.offset_mut();

        if let Some(match_flat) = state.tree_search_match_index {
            let windowed_match = entries
                .iter()
                .position(|e| e.real_index == Some(match_flat));
            if let Some(wm) = windowed_match {
                if Some(wm) != windowed_selected {
                    let offset = *temp_list_state.offset_mut();
                    let visible_height = inner_area.height as usize;
                    if wm >= offset && wm < offset + visible_height {
                        let row_y = inner_area.y + (wm - offset) as u16;
                        let match_style =
                            Style::new().bg(Color::Rgb(200, 150, 0)).fg(Color::Black);
                        for x in inner_area.x..inner_area.x + inner_area.width {
                            buf[(x, row_y)].set_style(match_style);
                        }
                    }
                }
            }
        }

        let scrollbar = scrollbar(ScrollbarOrientation::VerticalRight);
        StatefulWidget::render(
            scrollbar,
            inner_area,
            buf,
            &mut ScrollbarState::new(self.work_tree_root.len())
                .position(state.list_state.selected().unwrap_or_default()),
        );

        if let (Some(search_y), Some(ts)) = (search_bar_y, &self.tree_search_state) {
            let search_x = area.x;
            let width = area.width as usize;
            let left = if ts.is_input_mode {
                format!("?{}█", ts.query)
            } else {
                format!("?{}", ts.query)
            };
            let no_result = !ts.is_input_mode
                && ts.matches.is_empty()
                && (ts.bfs_queue.is_empty() || ts.exhausted);
            let right = if !ts.matches.is_empty() {
                format!("{}/{}", ts.current_match + 1, ts.matches.len())
            } else if no_result {
                String::from("no result")
            } else if !ts.is_input_mode {
                String::from("0/0")
            } else {
                String::new()
            };
            let padding = width.saturating_sub(left.len() + right.len());
            let display = format!("{}{:padding$}{}", left, "", right);
            let display = if display.len() > width {
                &display[..width]
            } else {
                &display
            };
            let style = if no_result {
                Style::new().fg(Color::Rgb(239, 68, 68))
            } else {
                Style::new()
            };
            buf.set_string(search_x, search_y, display, style);
        }
    }
}

#[cfg(test)]
mod test {
    use byte_unit::Byte;
    use crossterm::event::{KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use insta::assert_snapshot;

    use crate::{
        app::component::test_render::stateful_render_to_string, container::node::NodeKind,
        fixtures::SAMPLE_JSON,
    };

    use super::*;

    #[test]
    fn event_handler_ignore_key_release_test() {
        let json = String::from("123");
        let worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());

        assert_event_to_action(
            &worktree,
            Event::Key(KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::empty(),
                kind: KeyEventKind::Release,
                state: KeyEventState::NONE,
            }),
            vec![],
        );
    }

    #[test]
    fn event_handler_navigation_test() {
        let json = String::from("123");
        let worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());

        for (key, action) in [
            ((KeyCode::Up, KeyModifiers::NONE), NavigationAction::Up(1)),
            (
                (KeyCode::Char('k'), KeyModifiers::NONE),
                NavigationAction::Up(1),
            ),
            (
                (KeyCode::Char('u'), KeyModifiers::CONTROL),
                NavigationAction::Up(10),
            ),
            (
                (KeyCode::Down, KeyModifiers::NONE),
                NavigationAction::Down(1),
            ),
            (
                (KeyCode::Char('j'), KeyModifiers::NONE),
                NavigationAction::Down(1),
            ),
            (
                (KeyCode::Char('d'), KeyModifiers::CONTROL),
                NavigationAction::Down(10),
            ),
            (
                (KeyCode::Char('l'), KeyModifiers::NONE),
                NavigationAction::Expand,
            ),
            (
                (KeyCode::Char(' '), KeyModifiers::NONE),
                NavigationAction::Expand,
            ),
            (
                (KeyCode::Tab, KeyModifiers::NONE),
                NavigationAction::Expand,
            ),
            (
                (KeyCode::Char('h'), KeyModifiers::NONE),
                NavigationAction::Close,
            ),
            (
                (KeyCode::BackTab, KeyModifiers::SHIFT),
                NavigationAction::CloseOrCloseParent,
            ),
            (
                (KeyCode::Char('p'), KeyModifiers::NONE),
                NavigationAction::TogglePreview,
            ),
            (
                (KeyCode::Char('K'), KeyModifiers::NONE),
                NavigationAction::PreviewNavigation(PreviewNavigationAction::Up(1)),
            ),
            (
                (KeyCode::Char('J'), KeyModifiers::NONE),
                NavigationAction::PreviewNavigation(PreviewNavigationAction::Down(1)),
            ),
            (
                (KeyCode::Char('U'), KeyModifiers::CONTROL),
                NavigationAction::PreviewNavigation(PreviewNavigationAction::Up(5)),
            ),
            (
                (KeyCode::Char('D'), KeyModifiers::CONTROL),
                NavigationAction::PreviewNavigation(PreviewNavigationAction::Down(5)),
            ),
            (
                (KeyCode::Char('H'), KeyModifiers::NONE),
                NavigationAction::PreviewNavigation(PreviewNavigationAction::Left),
            ),
            (
                (KeyCode::Char('L'), KeyModifiers::NONE),
                NavigationAction::PreviewNavigation(PreviewNavigationAction::Right),
            ),
            (
                (KeyCode::Left, KeyModifiers::CONTROL),
                NavigationAction::PreviewWindowResize(Op::Add(1)),
            ),
            (
                (KeyCode::Right, KeyModifiers::CONTROL),
                NavigationAction::PreviewWindowResize(Op::Sub(1)),
            ),
            (
                (KeyCode::Enter, KeyModifiers::NONE),
                NavigationAction::Search(SearchAction::Next),
            ),
            (
                (KeyCode::Char('/'), KeyModifiers::NONE),
                NavigationAction::Search(SearchAction::Start),
            ),
            (
                (KeyCode::Char('?'), KeyModifiers::NONE),
                NavigationAction::TreeSearch(TreeSearchAction::Start),
            ),
        ] {
            assert_key_event_to_action(&worktree, key, vec![action.into()]);
        }
    }

    #[test]
    fn event_handler_fileops_test() {
        let json = String::from("123");
        let worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());

        for (key, action) in [
            (
                (KeyCode::Char('q'), KeyModifiers::NONE),
                Action::Exit(ConfirmAction::Request(())),
            ),
            (
                (KeyCode::Char('e'), KeyModifiers::NONE),
                WorkSpaceAction::Edit.into(),
            ),
            (
                (KeyCode::Char('w'), KeyModifiers::NONE),
                WorkSpaceAction::Save(ConfirmAction::Request(())).into(),
            ),
        ] {
            assert_key_event_to_action(&worktree, key, vec![action]);
        }
    }

    #[test]
    fn event_handler_ignore_on_confirm_dialog() {
        let json = String::from("123");
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        let mut state = WorkSpaceState::default();
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Save(ConfirmAction::Request(())),
        );

        for key in [
            (KeyCode::Char('q'), KeyModifiers::NONE),
            (KeyCode::Char('e'), KeyModifiers::NONE),
            (KeyCode::Char('w'), KeyModifiers::NONE),
            (KeyCode::Char('k'), KeyModifiers::NONE),
            (KeyCode::Up, KeyModifiers::NONE),
        ] {
            assert_key_event_to_action(&worktree, key, Vec::new());
        }

        worktree.test_action(
            &mut state,
            WorkSpaceAction::Save(ConfirmAction::Confirm(false)),
        );
        assert_key_event_to_action(
            &worktree,
            (KeyCode::Up, KeyModifiers::NONE),
            vec![NavigationAction::Up(1).into()],
        );
    }

    #[test]
    fn handle_navigation_action() {
        let json = String::from(r#"{"key": "string", "values": [1, 2, 3]}"#);
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        let mut state = WorkSpaceState::default();
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(&mut state, NavigationAction::Down(1).into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        for _ in 0..3 {
            worktree.test_action(&mut state, NavigationAction::Up(1).into());
        }
        worktree.test_action(&mut state, NavigationAction::Close.into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn write_selected_test() {
        let json = String::from(r#"{"key": "string", "values": [1, 2, 3]}"#);
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        let mut state = WorkSpaceState::default();
        worktree.test_action(&mut state, NavigationAction::Expand.into());

        worktree.test_action(&mut state, NavigationAction::Down(1).into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Up(1).into());

        let mut buffer = Vec::new();
        worktree.write_selected(&state, &mut buffer).unwrap();
        assert_eq!(String::from_utf8(buffer).unwrap(), "[\n  1,\n  2,\n  3\n]",)
    }

    #[test]
    fn load_selected_test() {
        let json = String::from(r#"{"key": "string", "values": [1, 2, 3]}"#);
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        let mut state = WorkSpaceState::default();
        worktree.test_action(&mut state, NavigationAction::Expand.into());

        worktree.test_action(&mut state, NavigationAction::Down(1).into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Up(1).into());

        worktree.test_action(
            &mut state,
            WorkSpaceAction::Load {
                node: Node::load("[{}, 5]".as_bytes()).unwrap(),
                is_edit: true,
            },
        );

        assert_eq!(
            worktree.file_root().to_string_pretty().unwrap(),
            "{\n  \"key\": \"string\",\n  \"values\": [\n    {},\n    5\n  ]\n}"
        );
    }

    #[test]
    fn handle_edit_error_action_test() {
        let json = String::from("123");
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        let mut state = WorkSpaceState::default();

        let action = WorkSpaceAction::EditError(ConfirmAction::Request(String::from(
            "Deserialization error: expected value at line 1 column 2",
        )));
        assert!(worktree.test_action(&mut state, action.clone()).is_empty());
        assert_eq!(worktree.dialogs.len(), 1);
        assert!(
            worktree
                .test_action(
                    &mut state,
                    WorkSpaceAction::EditError(ConfirmAction::Confirm(false))
                )
                .is_empty()
        );
        assert!(worktree.dialogs.is_empty());

        assert!(worktree.test_action(&mut state, action.clone()).is_empty());
        assert_eq!(worktree.dialogs.len(), 1);
        assert_eq!(
            worktree.test_action(
                &mut state,
                WorkSpaceAction::EditError(ConfirmAction::Confirm(true))
            ),
            vec![JobAction::Edit(EditJobAction::Open).into()]
        );
        assert!(worktree.dialogs.is_empty());
    }

    #[test]
    fn event_handler_dialog_test() {
        let json = String::from("123");
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        let mut state = WorkSpaceState::default();

        worktree.test_action(
            &mut state,
            WorkSpaceAction::EditError(ConfirmAction::Request(String::from(
                "Deserialization error: expected value at line 1 column 2",
            ))),
        );
        assert_key_event_to_action(
            &worktree,
            (KeyCode::Char('y'), KeyModifiers::NONE),
            vec![WorkSpaceAction::EditError(ConfirmAction::Confirm(true)).into()],
        );
    }

    #[test]
    fn render_edit_error_test() {
        let json = String::from("123");
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        let mut state = WorkSpaceState::default();

        for response in [true, false] {
            worktree.test_action(
                &mut state,
                WorkSpaceAction::EditError(ConfirmAction::Request(String::from(
                    "Deserialization error: expected value at line 1 column 2",
                ))),
            );
            if response {
                assert_snapshot!(stateful_render_to_string(
                    &worktree,
                    &mut WorkSpaceState::default()
                ));
            }

            worktree.handle_edit_error_action(ConfirmAction::Confirm(response));
            assert_snapshot!(stateful_render_to_string(
                &worktree,
                &mut WorkSpaceState::default()
            ));
        }
    }

    #[test]
    fn render_edit_error_long_message_test() {
        let json = String::from("123");
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, WorkSpaceAction::EditError(ConfirmAction::Request(String::from(
            concat!(
                "Deserialization error: expected value at line 1 column 2. Lorem ipsum dolor sit amet,",
                "consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna",
                "aliqua.",
            )
        ))));

        assert_snapshot!(stateful_render_to_string(
            &worktree,
            &mut WorkSpaceState::default()
        ));
    }

    #[test]
    fn exit_without_change_test() {
        let json = String::from("123");
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        assert!(worktree.maybe_exit(ConfirmAction::Request(())));

        let mut state = WorkSpaceState::default();
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Load {
                node: Node::load(String::from("456").as_bytes()).unwrap(),
                is_edit: true,
            },
        );
        assert!(!worktree.maybe_exit(ConfirmAction::Request(())));
        assert!(!worktree.maybe_exit(ConfirmAction::Confirm(false)));

        worktree.test_action(
            &mut state,
            WorkSpaceAction::Load {
                node: Node::load(String::from("123").as_bytes()).unwrap(),
                is_edit: true,
            },
        );
        assert!(!worktree.maybe_exit(ConfirmAction::Request(())));
        assert!(worktree.maybe_exit(ConfirmAction::Confirm(true)));

        worktree.test_action(
            &mut state,
            WorkSpaceAction::Load {
                node: Node::load(String::from("123").as_bytes()).unwrap(),
                is_edit: true,
            },
        );
        worktree.handle_save_done();
        assert!(worktree.maybe_exit(ConfirmAction::Request(())));
    }

    #[test]
    fn render_exit_confirm_test() {
        let json = String::from("123");
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());

        let mut state = WorkSpaceState::default();
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Load {
                node: Node::load(String::from("456").as_bytes()).unwrap(),
                is_edit: true,
            },
        );
        assert!(!worktree.maybe_exit(ConfirmAction::Request(())));

        assert_snapshot!(stateful_render_to_string(&worktree, &mut state,));
    }

    #[test]
    fn render_save_dialog_test() {
        let json = String::from("123");
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());

        let mut state = WorkSpaceState::default();
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Save(ConfirmAction::Request(())),
        );

        assert_snapshot!(stateful_render_to_string(&worktree, &mut state,));
    }

    #[test]
    fn render_preview_test() {
        let json = serde_json::to_string_pretty(&serde_json::json!({
            "key": "value",
            "array": [1, 2, ["cat", "dog"]]
        }))
        .unwrap();
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::TogglePreview.into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(&mut state, NavigationAction::Expand.into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(&mut state, NavigationAction::Down(1).into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(&mut state, NavigationAction::TogglePreview.into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn preview_out_of_bound_test() {
        let json = serde_json::to_string_pretty(&serde_json::json!({
            "key": "value",
            "array": [1, 2, ["cat", "dog"]]
        }))
        .unwrap();
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        let mut state = WorkSpaceState::default();

        for action in [
            NavigationAction::TogglePreview,
            NavigationAction::Up(1),
            NavigationAction::Expand,
            NavigationAction::Down(1),
            NavigationAction::Down(1),
            NavigationAction::Up(1),
        ] {
            worktree.test_action(&mut state, action.into());
        }

        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_preview_scroll_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default(),
        );
        let mut state = WorkSpaceState::default();

        for action in [NavigationAction::TogglePreview, NavigationAction::Expand] {
            worktree.test_action(&mut state, action.into());
        }

        for action in [
            PreviewNavigationAction::Up(1),
            PreviewNavigationAction::Down(1),
            PreviewNavigationAction::Down(1),
            PreviewNavigationAction::Up(1),
            PreviewNavigationAction::Right,
            PreviewNavigationAction::Right,
            PreviewNavigationAction::Left,
        ] {
            worktree.test_action(&mut state, action.into());
            assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
        }
    }

    #[test]
    fn render_preview_overflow_scroll_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default(),
        );
        let mut state = WorkSpaceState::default();

        for action in [NavigationAction::TogglePreview, NavigationAction::Expand] {
            worktree.test_action(&mut state, action.into());
        }

        for action in [
            PreviewNavigationAction::Down(3),
            PreviewNavigationAction::Down(100),
            PreviewNavigationAction::Up(3),
            PreviewNavigationAction::Up(100),
        ] {
            worktree.test_action(&mut state, action.into());
            assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
        }
    }

    #[test]
    fn render_preview_update_on_edit_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default(),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::TogglePreview.into());
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Load {
                node: Node::load("123".as_bytes()).unwrap(),
                is_edit: true,
            },
        );

        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_preview_overlap_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default(),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::TogglePreview.into());
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Load {
                node: Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
                is_edit: true,
            },
        );
        worktree.maybe_exit(ConfirmAction::Request(()));
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.maybe_exit(ConfirmAction::Confirm(false));
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn meta_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default(),
        );

        assert_eq!(
            worktree.meta_on_index(0),
            NodeMeta {
                n_lines: 100,
                n_bytes: 3718,
                kind: NodeKind::Object,
            }
        );
    }

    #[test]
    fn render_loading_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default(),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::TogglePreview.into());
        worktree.set_loading(true);
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.set_loading(false);
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_large_preview_test() {
        let json_bodies: Vec<_> = std::iter::repeat_n(SAMPLE_JSON, 1024).collect();
        let json = String::from("[") + &json_bodies.join(",") + "]";
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::TogglePreview.into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(&mut state, NavigationAction::Expand.into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(&mut state, NavigationAction::Up(1).into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_preview_limited_by_config_test() {
        let config = Config::default().with_max_preview_size(Byte::from_u64(3718));
        let mut worktree = WorkSpace::new(Node::load(SAMPLE_JSON.as_bytes()).unwrap(), config);
        assert_eq!(worktree.file_root.to_string_pretty().unwrap().len(), 3718);
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::TogglePreview.into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        let config = Config::default().with_max_preview_size(Byte::from_u64(3717));
        let mut worktree = WorkSpace::new(Node::load(SAMPLE_JSON.as_bytes()).unwrap(), config);
        worktree.test_action(&mut state, NavigationAction::TogglePreview.into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_navigation_far_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default(),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::TogglePreview.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Down(2).into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Down(10).into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
        worktree.test_action(&mut state, NavigationAction::Down(100).into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
        worktree.test_action(&mut state, NavigationAction::Up(100).into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_preview_resize_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default(),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::TogglePreview.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
        worktree.test_action(
            &mut state,
            NavigationAction::PreviewWindowResize(Op::Sub(1)).into(),
        );
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
        worktree.test_action(
            &mut state,
            NavigationAction::PreviewWindowResize(Op::Add(3)).into(),
        );
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
        worktree.test_action(
            &mut state,
            NavigationAction::PreviewWindowResize(Op::Sub(100)).into(),
        );
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
        worktree.test_action(
            &mut state,
            NavigationAction::PreviewWindowResize(Op::Add(100)).into(),
        );
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_top_bottom_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default(),
        );
        let mut state = WorkSpaceState::default();

        for _ in 0..4 {
            worktree.test_action(&mut state, NavigationAction::Expand.into());
        }
        worktree.test_action(&mut state, NavigationAction::Down(2).into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());

        worktree.test_action(&mut state, NavigationAction::Bottom.into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(&mut state, NavigationAction::Top.into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_top_bottom_preview_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default(),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::TogglePreview.into());
        for _ in 0..4 {
            worktree.test_action(&mut state, NavigationAction::Expand.into());
        }
        worktree.test_action(&mut state, NavigationAction::Down(2).into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());

        worktree.test_action(&mut state, NavigationAction::Bottom.into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(&mut state, NavigationAction::Top.into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_bottom_select_last_index_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default(),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::Bottom.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(&mut state, NavigationAction::Bottom.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_rename_root_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default(),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(
            &mut state,
            WorkSpaceAction::Rename(ConfirmAction::Request(())),
        );
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_invalid_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default(),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Rename(ConfirmAction::Request(())),
        );
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_rename_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default(),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Rename(ConfirmAction::Request(())),
        );
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(
            &mut state,
            WorkSpaceAction::Rename(ConfirmAction::Confirm(None)),
        );
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(
            &mut state,
            WorkSpaceAction::Rename(ConfirmAction::Request(())),
        );
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Rename(ConfirmAction::Confirm(Some(String::from("new_key")))),
        );
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(
            &mut state,
            WorkSpaceAction::Rename(ConfirmAction::Request(())),
        );
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_rename_duplicate_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default(),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Rename(ConfirmAction::Request(())),
        );
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Rename(ConfirmAction::Confirm(Some(String::from("taglib")))),
        );
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_rename_does_not_change_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default(),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Rename(ConfirmAction::Request(())),
        );
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Rename(ConfirmAction::Confirm(Some(String::from("servlet")))),
        );
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_simple_delete_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default(),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Delete(ConfirmAction::Request(())),
        );
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Delete(ConfirmAction::Confirm(true)),
        );
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(&mut state, NavigationAction::Down(2).into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_delete_rename_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default(),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Delete(ConfirmAction::Request(())),
        );
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Delete(ConfirmAction::Confirm(true)),
        );
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Rename(ConfirmAction::Request(())),
        );
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Rename(ConfirmAction::Confirm(Some(String::from("new_key")))),
        );

        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_delete_preview_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default(),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::TogglePreview.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Delete(ConfirmAction::Request(())),
        );
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Delete(ConfirmAction::Confirm(true)),
        );

        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_delete_and_go_to_bottom_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default(),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Delete(ConfirmAction::Request(())),
        );
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Delete(ConfirmAction::Confirm(true)),
        );

        worktree.test_action(&mut state, NavigationAction::Bottom.into());
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Delete(ConfirmAction::Request(())),
        );
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Delete(ConfirmAction::Confirm(true)),
        );
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_delete_large_preview_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default().with_max_preview_size(Byte::from_u64(3700)),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::TogglePreview.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Delete(ConfirmAction::Request(())),
        );
        worktree.test_action(
            &mut state,
            WorkSpaceAction::Delete(ConfirmAction::Confirm(true)),
        );

        worktree.test_action(&mut state, NavigationAction::Top.into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_append_after_into_array_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default().with_max_preview_size(Byte::from_u64(3700)),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::TogglePreview.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, WorkSpaceAction::Add(ConfirmAction::Request(())));
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(&mut state, NavigationAction::Down(1).into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(&mut state, NavigationAction::Down(4).into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_append_after_into_object_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default().with_max_preview_size(Byte::from_u64(3700)),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::TogglePreview.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, WorkSpaceAction::Add(ConfirmAction::Request(())));
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(
            &mut state,
            WorkSpaceAction::Add(ConfirmAction::Confirm(None)),
        );
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(
            &mut state,
            WorkSpaceAction::Add(ConfirmAction::Confirm(Some(String::from("new_key")))),
        );
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(&mut state, NavigationAction::Down(1).into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(&mut state, NavigationAction::Down(3).into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_append_after_into_object_key_exists_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default().with_max_preview_size(Byte::from_u64(3700)),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::TogglePreview.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, WorkSpaceAction::Add(ConfirmAction::Request(())));

        worktree.test_action(
            &mut state,
            WorkSpaceAction::Add(ConfirmAction::Confirm(Some(String::from("taglib")))),
        );
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(&mut state, WorkSpaceAction::ErrorConfirmed);
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        worktree.test_action(
            &mut state,
            WorkSpaceAction::Add(ConfirmAction::Confirm(Some(String::from("taglib2")))),
        );
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn render_test_expand_expanded_test() {
        let mut worktree = WorkSpace::new(
            Node::load(SAMPLE_JSON.as_bytes()).unwrap(),
            Config::default().with_max_preview_size(Byte::from_u64(3700)),
        );
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::TogglePreview.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Top.into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    #[test]
    fn handle_close_or_close_parent_test() {
        let json = String::from(r#"{"key": "string", "values": [1, 2, 3]}"#);
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        let mut state = WorkSpaceState::default();

        // Expand root, move into "values", expand it
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Down(1).into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());

        // Now on first child of "values" (index 3), which is not expanded
        assert_eq!(state.list_state.selected(), Some(3));

        // CloseOrCloseParent on unexpanded node: should close parent "values" and move cursor to it
        worktree.test_action(&mut state, NavigationAction::CloseOrCloseParent.into());
        assert_eq!(state.list_state.selected(), Some(2));
        assert!(!worktree.work_tree_root.is_expanded(2)); // "values" is now closed

        // Now on "values" (index 2), which is not expanded
        // CloseOrCloseParent should close parent "root" and move cursor to it
        worktree.test_action(&mut state, NavigationAction::CloseOrCloseParent.into());
        assert_eq!(state.list_state.selected(), Some(0));
        assert!(!worktree.work_tree_root.is_expanded(0)); // root is now closed

        // Re-expand root and "values"
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Down(1).into());
        worktree.test_action(&mut state, NavigationAction::Expand.into());
        worktree.test_action(&mut state, NavigationAction::Up(1).into());

        // Now on "values" (index 2), which IS expanded
        assert_eq!(state.list_state.selected(), Some(2));
        assert!(worktree.work_tree_root.is_expanded(2));

        // CloseOrCloseParent on expanded node: should close current, stay on same index
        worktree.test_action(&mut state, NavigationAction::CloseOrCloseParent.into());
        assert_eq!(state.list_state.selected(), Some(2));
        assert!(!worktree.work_tree_root.is_expanded(2));
    }

    #[test]
    fn search_event_routing_test() {
        let json = String::from("123");
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        worktree.search_active = true;

        for (key, action) in [
            (
                (KeyCode::Char('a'), KeyModifiers::NONE),
                SearchAction::Input('a'),
            ),
            (
                (KeyCode::Char('z'), KeyModifiers::NONE),
                SearchAction::Input('z'),
            ),
            (
                (KeyCode::Backspace, KeyModifiers::NONE),
                SearchAction::Backspace,
            ),
            (
                (KeyCode::Enter, KeyModifiers::NONE),
                SearchAction::Confirm,
            ),
            ((KeyCode::Esc, KeyModifiers::NONE), SearchAction::Cancel),
        ] {
            assert_key_event_to_action(&worktree, key, vec![action.into()]);
        }

        // Verify normal keys are NOT routed when search_active
        assert_key_event_to_action(
            &worktree,
            (KeyCode::Char('q'), KeyModifiers::NONE),
            vec![SearchAction::Input('q').into()],
        );

        // Verify n/p route to Next/Previous when search_has_results (not in input mode)
        let mut worktree2 =
            WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        worktree2.search_has_results = true;
        assert_key_event_to_action(
            &worktree2,
            (KeyCode::Char('n'), KeyModifiers::NONE),
            vec![SearchAction::Next.into()],
        );
        assert_key_event_to_action(
            &worktree2,
            (KeyCode::Char('p'), KeyModifiers::NONE),
            vec![SearchAction::Previous.into()],
        );

        // Verify p routes to TogglePreview when no search results
        let worktree3 =
            WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        assert_key_event_to_action(
            &worktree3,
            (KeyCode::Char('p'), KeyModifiers::NONE),
            vec![NavigationAction::TogglePreview.into()],
        );
    }

    #[test]
    fn handle_search_test() {
        let json = serde_json::to_string_pretty(&serde_json::json!({
            "key": "value",
            "array": [1, 2, ["cat", "dog"]]
        }))
        .unwrap();
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        let mut state = WorkSpaceState::default();

        // Enable preview
        worktree.test_action(&mut state, NavigationAction::TogglePreview.into());

        // Start search
        worktree.test_action(&mut state, SearchAction::Start.into());
        assert!(worktree.search_active);
        assert!(state.preview_state.search.as_ref().unwrap().is_input_mode);

        // Input chars
        worktree.test_action(&mut state, SearchAction::Input('c').into());
        worktree.test_action(&mut state, SearchAction::Input('a').into());
        worktree.test_action(&mut state, SearchAction::Input('t').into());
        assert_eq!(state.preview_state.search.as_ref().unwrap().query, "cat");

        // Backspace
        worktree.test_action(&mut state, SearchAction::Backspace.into());
        assert_eq!(state.preview_state.search.as_ref().unwrap().query, "ca");

        // Re-add and confirm
        worktree.test_action(&mut state, SearchAction::Input('t').into());
        worktree.test_action(&mut state, SearchAction::Confirm.into());
        assert!(!worktree.search_active);
        let search = state.preview_state.search.as_ref().unwrap();
        assert!(!search.is_input_mode);
        assert!(!search.matches.is_empty());
        assert_eq!(search.current_match, 0);

        // Snapshot with search highlights
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        // Next at last match stays at last match (no wrap)
        assert!(worktree.search_has_results);
        let n_matches = state.preview_state.search.as_ref().unwrap().matches.len();
        assert_eq!(n_matches, 1);
        worktree.test_action(&mut state, SearchAction::Next.into());
        assert_eq!(state.preview_state.search.as_ref().unwrap().current_match, 0);

        // Previous at first match stays at first match (no wrap)
        worktree.test_action(&mut state, SearchAction::Previous.into());
        assert_eq!(state.preview_state.search.as_ref().unwrap().current_match, 0);

        // Cancel clears search
        worktree.test_action(&mut state, SearchAction::Start.into());
        worktree.test_action(&mut state, SearchAction::Cancel.into());
        assert!(!worktree.search_active);
        assert!(!worktree.search_has_results);
        assert!(state.preview_state.search.is_none());
    }

    #[test]
    fn render_search_bar_test() {
        let json = serde_json::to_string_pretty(&serde_json::json!({
            "key": "value",
            "array": [1, 2, ["cat", "dog"]]
        }))
        .unwrap();
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, NavigationAction::TogglePreview.into());
        worktree.test_action(&mut state, SearchAction::Start.into());
        worktree.test_action(&mut state, SearchAction::Input('v').into());
        worktree.test_action(&mut state, SearchAction::Input('a').into());
        worktree.test_action(&mut state, SearchAction::Input('l').into());

        // Snapshot showing search bar with /val█
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }

    fn assert_key_event_to_action(
        worktree: &WorkSpace,
        (code, modifiers): (KeyCode, KeyModifiers),
        expected_actions: Vec<Action>,
    ) {
        assert_event_to_action(
            worktree,
            Event::Key(KeyEvent {
                code,
                modifiers,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }),
            expected_actions,
        );
    }

    fn assert_event_to_action(worktree: &WorkSpace, event: Event, expected_actions: Vec<Action>) {
        let mut actions = Actions::new();
        worktree.handle_event(&mut actions, event);
        assert_eq!(actions.into_vec(), expected_actions)
    }

    impl WorkSpace {
        fn test_action(
            &mut self,
            state: &mut WorkSpaceState,
            action: WorkSpaceAction,
        ) -> Vec<Action> {
            let mut actions = Actions::new();
            self.handle_action(state, &mut actions, action).unwrap();
            actions.into_vec()
        }

        pub fn write_selected(
            &self,
            worktree_state: &WorkSpaceState,
            writer: impl Write,
        ) -> std::io::Result<bool> {
            let Some(index) = worktree_state.list_state.selected() else {
                return Ok(false);
            };
            self.write_on_index(writer, index)?;

            Ok(true)
        }
    }

    #[test]
    fn tree_search_event_routing_test() {
        let json = String::from("123");
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        worktree.tree_search_active = true;

        for (key, action) in [
            (
                (KeyCode::Char('a'), KeyModifiers::NONE),
                TreeSearchAction::Input('a'),
            ),
            (
                (KeyCode::Char('z'), KeyModifiers::NONE),
                TreeSearchAction::Input('z'),
            ),
            (
                (KeyCode::Backspace, KeyModifiers::NONE),
                TreeSearchAction::Backspace,
            ),
            (
                (KeyCode::Enter, KeyModifiers::NONE),
                TreeSearchAction::Confirm,
            ),
            (
                (KeyCode::Esc, KeyModifiers::NONE),
                TreeSearchAction::Cancel,
            ),
        ] {
            assert_key_event_to_action(&worktree, key, vec![action.into()]);
        }

        // Verify normal keys are NOT routed when tree_search_active
        assert_key_event_to_action(
            &worktree,
            (KeyCode::Char('q'), KeyModifiers::NONE),
            vec![TreeSearchAction::Input('q').into()],
        );

        // Verify n/p route to TreeSearchAction when tree_search_has_results
        let mut worktree2 =
            WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        worktree2.tree_search_has_results = true;
        assert_key_event_to_action(
            &worktree2,
            (KeyCode::Char('n'), KeyModifiers::NONE),
            vec![TreeSearchAction::Next.into()],
        );
        assert_key_event_to_action(
            &worktree2,
            (KeyCode::Char('p'), KeyModifiers::NONE),
            vec![TreeSearchAction::Previous.into()],
        );
    }

    #[test]
    fn tree_search_bfs_order_test() {
        // {"a":{"target":1},"target":2} — should find top-level "target" first
        let json = r#"{"a":{"target":1},"target":2}"#;
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        let mut state = WorkSpaceState::default();

        // Start tree search
        worktree.test_action(&mut state, TreeSearchAction::Start.into());
        worktree.test_action(&mut state, TreeSearchAction::Input('t').into());
        worktree.test_action(&mut state, TreeSearchAction::Input('a').into());
        worktree.test_action(&mut state, TreeSearchAction::Input('r').into());
        worktree.test_action(&mut state, TreeSearchAction::Input('g').into());
        worktree.test_action(&mut state, TreeSearchAction::Input('e').into());
        worktree.test_action(&mut state, TreeSearchAction::Input('t').into());
        worktree.test_action(&mut state, TreeSearchAction::Confirm.into());

        // First match should be top-level "target" (shallowest)
        let ts = worktree.tree_search_state.as_ref().unwrap();
        assert_eq!(ts.matches.len(), 1);
        assert_eq!(ts.matches[0], vec!["target"]);
        assert!(worktree.tree_search_has_results);

        // Next should find nested "target"
        worktree.test_action(&mut state, TreeSearchAction::Next.into());
        let ts = worktree.tree_search_state.as_ref().unwrap();
        assert_eq!(ts.matches.len(), 2);
        assert_eq!(ts.matches[1], vec!["a", "target"]);
    }

    #[test]
    fn tree_search_through_array_test() {
        // {"list":[{"target":1}]} — should find "target" inside array element
        let json = r#"{"list":[{"target":1}]}"#;
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, TreeSearchAction::Start.into());
        for c in "target".chars() {
            worktree.test_action(&mut state, TreeSearchAction::Input(c).into());
        }
        worktree.test_action(&mut state, TreeSearchAction::Confirm.into());

        let ts = worktree.tree_search_state.as_ref().unwrap();
        assert_eq!(ts.matches.len(), 1);
        assert_eq!(ts.matches[0], vec!["list", "0", "target"]);
        assert!(worktree.tree_search_has_results);

        // Tree should be expanded to "target"
        let selector = worktree.work_tree_root.selector(state.list_state.selected().unwrap());
        assert_eq!(selector, vec!["list", "0", "target"]);
    }

    #[test]
    fn tree_search_next_previous_test() {
        // {"a":"x","b":"y","ab":"z"} — "a" matches "a" and "ab"
        let json = r#"{"a":"x","b":"y","ab":"z"}"#;
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, TreeSearchAction::Start.into());
        worktree.test_action(&mut state, TreeSearchAction::Input('a').into());
        worktree.test_action(&mut state, TreeSearchAction::Confirm.into());

        let ts = worktree.tree_search_state.as_ref().unwrap();
        assert_eq!(ts.current_match, 0);
        assert_eq!(ts.matches[0], vec!["a"]);

        // next resumes BFS, should find "ab"
        worktree.test_action(&mut state, TreeSearchAction::Next.into());
        let ts = worktree.tree_search_state.as_ref().unwrap();
        assert_eq!(ts.matches.len(), 2);
        assert_eq!(ts.current_match, 1);
        assert_eq!(ts.matches[1], vec!["ab"]);

        // previous goes back
        worktree.test_action(&mut state, TreeSearchAction::Previous.into());
        let ts = worktree.tree_search_state.as_ref().unwrap();
        assert_eq!(ts.current_match, 0);

        // previous at 0 stays at 0
        worktree.test_action(&mut state, TreeSearchAction::Previous.into());
        let ts = worktree.tree_search_state.as_ref().unwrap();
        assert_eq!(ts.current_match, 0);
    }

    #[test]
    fn tree_search_cancel_test() {
        let json = r#"{"a":1,"b":2}"#;
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, TreeSearchAction::Start.into());
        assert!(worktree.tree_search_active);
        assert!(worktree.tree_search_state.is_some());

        worktree.test_action(&mut state, TreeSearchAction::Cancel.into());
        assert!(!worktree.tree_search_active);
        assert!(!worktree.tree_search_has_results);
        assert!(worktree.tree_search_state.is_none());
    }

    #[test]
    fn tree_search_no_match_test() {
        let json = r#"{"a":1,"b":2}"#;
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        let mut state = WorkSpaceState::default();

        worktree.test_action(&mut state, TreeSearchAction::Start.into());
        for c in "zzz".chars() {
            worktree.test_action(&mut state, TreeSearchAction::Input(c).into());
        }
        worktree.test_action(&mut state, TreeSearchAction::Confirm.into());

        let ts = worktree.tree_search_state.as_ref().unwrap();
        assert!(ts.matches.is_empty());
        assert!(!worktree.tree_search_has_results);
    }

    #[test]
    fn render_tree_search_bar_test() {
        let json = r#"{"key":"value","array":[1,2,3]}"#;
        let mut worktree = WorkSpace::new(Node::load(json.as_bytes()).unwrap(), Config::default());
        let mut state = WorkSpaceState::default();

        // Start tree search and type
        worktree.test_action(&mut state, TreeSearchAction::Start.into());
        worktree.test_action(&mut state, TreeSearchAction::Input('k').into());
        worktree.test_action(&mut state, TreeSearchAction::Input('e').into());
        worktree.test_action(&mut state, TreeSearchAction::Input('y').into());

        // Input mode: ?key█
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));

        // Confirm search
        worktree.test_action(&mut state, TreeSearchAction::Confirm.into());

        // Confirmed mode: ?key with match counter
        assert_snapshot!(stateful_render_to_string(&worktree, &mut state));
    }
}
