use std::{cell::RefCell, collections::BTreeSet, iter::Peekable, slice::Iter};

use crate::container::node::{Index, IndexKind, NodeKind, NodeMeta};

const WINDOW_THRESHOLD: usize = 12;
const EDGE_COUNT: usize = 3;
const FOCUS_RADIUS: usize = 3;

#[derive(Debug)]
pub struct WindowedTreeEntry {
    pub display: String,
    pub real_index: Option<usize>,
}

#[derive(Debug)]
pub struct WorkTreeNode {
    name: String,
    len: usize,
    meta: Option<NodeMeta>,
    child: Option<Vec<WorkTreeNode>>,
}

impl WorkTreeNode {
    pub fn new(name: String, meta: Option<NodeMeta>) -> Self {
        Self {
            name,
            len: 1,
            meta,
            child: None,
        }
    }

    pub fn new_empty(name: String) -> Self {
        Self {
            name,
            len: 1,
            meta: None,
            child: None,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn as_tree_string(&self) -> impl Iterator<Item = String> {
        std::iter::once(self.formatted_name(Vec::new()))
            .chain(WorkTreeStringIter::new(self.child.as_deref()))
    }

    pub fn selector(&self, index: usize) -> Vec<&str> {
        let mut res = Vec::new();

        self.traverse_node(
            index,
            &mut |node| {
                if !std::ptr::eq(self, node) {
                    res.push(node.name.as_str());
                }
            },
            &mut |_| {},
            |_| {},
        );

        res
    }

    pub fn is_expanded(&self, index: usize) -> bool {
        self.traverse_node(index, &mut |_| {}, &mut |_| {}, |node| node.child.is_some())
    }

    pub fn reindex(&mut self, index: usize, node_index: Index, force: bool) {
        let (len, child) = match node_index.kind {
            IndexKind::Terminal => (1, Vec::new()),
            IndexKind::Object(items) => (
                items.len() + 1,
                items.into_iter().map(WorkTreeNode::new_empty).collect(),
            ),
            IndexKind::Array(n) => (
                n + 1,
                (0..n)
                    .map(|i| WorkTreeNode::new_empty(i.to_string()))
                    .collect(),
            ),
        };

        let old_len = RefCell::new(None);

        self.traverse_node_mut(
            index,
            &mut |_| {},
            &mut |node: &mut WorkTreeNode, _| {
                if let Some(old_len) = *old_len.borrow() {
                    node.len -= old_len;
                    node.len += len;
                }
            },
            |node: &mut WorkTreeNode| {
                node.meta = Some(node_index.meta);
                if node.child.is_some() || force {
                    *old_len.borrow_mut() = Some(node.len);
                    node.child = Some(child);
                }
            },
        );
    }

    pub(crate) fn rename(&mut self, index: usize, new_key: String) {
        let new_key_len = new_key.len();
        let old_key_len = RefCell::new(0);
        self.traverse_node_mut(
            index,
            &mut |_| {},
            &mut |node: &mut WorkTreeNode, _| {
                if let Some(meta) = &mut node.meta {
                    meta.n_bytes -= *old_key_len.borrow();
                    meta.n_bytes += new_key_len;
                }
            },
            |node: &mut WorkTreeNode| {
                *old_key_len.borrow_mut() = node.name.len();
                node.name = new_key;
            },
        );
    }

    pub(crate) fn delete(&mut self, index: usize, mut parent_metas: Vec<NodeMeta>) {
        let should_delete = RefCell::new(true);
        self.traverse_node_mut(
            index,
            &mut |_| {},
            &mut |node: &mut WorkTreeNode, child_index| {
                if *should_delete.borrow() {
                    let (Some(child), Some(child_index)) = (&mut node.child, child_index) else {
                        return;
                    };
                    child.remove(child_index);
                    let Some(meta) = node.meta else {
                        return;
                    };

                    if matches!(meta.kind, NodeKind::Array) {
                        for (index, child) in child.iter_mut().enumerate() {
                            child.name = index.to_string();
                        }
                    }
                    *should_delete.borrow_mut() = false;
                }

                if !*should_delete.borrow() {
                    node.len -= 1;
                    node.meta = Some(parent_metas.pop().expect("missing parent meta"));
                }
            },
            |_| {},
        );
    }

    pub(crate) fn append_after(
        &mut self,
        index: usize,
        key: Option<String>,
        mut parent_metas: Vec<NodeMeta>,
    ) {
        let should_append = RefCell::new(true);
        self.traverse_node_mut(
            index,
            &mut |_| {},
            &mut |node: &mut WorkTreeNode, child_index| {
                if *should_append.borrow() {
                    let (Some(child), Some(child_index)) = (&mut node.child, child_index) else {
                        return;
                    };
                    child.insert(
                        child_index + 1,
                        Self::new(key.clone().unwrap_or_default(), None),
                    );
                    let Some(meta) = node.meta else {
                        return;
                    };

                    if matches!(meta.kind, NodeKind::Array) {
                        for (index, child) in child.iter_mut().enumerate() {
                            child.name = index.to_string();
                        }
                    }
                    *should_append.borrow_mut() = false;
                }

                if !*should_append.borrow() {
                    node.len += 1;
                    node.meta = Some(parent_metas.pop().expect("missing parent meta"));
                }
            },
            |_| {},
        );
    }

    pub fn close(&mut self, index: usize) {
        let old_len = RefCell::new(1);
        self.traverse_node_mut(
            index,
            &mut |_| {},
            &mut |node: &mut WorkTreeNode, _| {
                node.len -= *old_len.borrow();
            },
            |node: &mut WorkTreeNode| {
                *old_len.borrow_mut() = node.len - 1;
                node.child = None;
            },
        );
    }

    pub fn meta(&self, index: usize) -> Option<NodeMeta> {
        self.traverse_node(index, &mut |_| {}, &mut |_| {}, |node| node.meta)
    }

    fn traverse_node<'a, B, A, F, R>(
        &'a self,
        mut index: usize,
        before_visit_hook: &mut B,
        after_visit_hook: &mut A,
        on_found_hook: F,
    ) -> R
    where
        B: FnMut(&'a WorkTreeNode),
        A: FnMut(&'a WorkTreeNode),
        F: FnOnce(&'a WorkTreeNode) -> R,
    {
        before_visit_hook(self);
        if index == 0 {
            let res = on_found_hook(self);
            after_visit_hook(self);
            return res;
        }

        if index >= self.len {
            panic!("unexpected index");
        }

        index -= 1;
        let child = self.child.as_deref().into_iter().flatten();
        for child in child {
            if index < child.len {
                let res =
                    child.traverse_node(index, before_visit_hook, after_visit_hook, on_found_hook);
                after_visit_hook(self);
                return res;
            }

            index -= child.len;
        }

        unreachable!()
    }

    fn traverse_node_mut<B, A, F>(
        &mut self,
        mut index: usize,
        before_visit_hook: &mut B,
        after_visit_hook: &mut A,
        on_found_hook: F,
    ) where
        B: FnMut(&mut WorkTreeNode),
        A: FnMut(&mut WorkTreeNode, Option<usize>),
        F: FnOnce(&mut WorkTreeNode),
    {
        before_visit_hook(self);
        if index == 0 {
            on_found_hook(self);
            after_visit_hook(self, None);
            return;
        }

        if index >= self.len {
            panic!("unexpected index");
        }

        index -= 1;
        let child = self.child.as_deref_mut().into_iter().flatten();
        for (child_index, child) in child.enumerate() {
            if index < child.len {
                child.traverse_node_mut(index, before_visit_hook, after_visit_hook, on_found_hook);
                after_visit_hook(self, Some(child_index));
                return;
            }

            index -= child.len;
        }

        unreachable!()
    }

    pub fn direct_children(&self, index: usize) -> Option<Vec<(usize, &str)>> {
        self.traverse_node(index, &mut |_| {}, &mut |_| {}, |node| {
            let children = node.child.as_ref()?;
            let mut result = Vec::new();
            let mut offset = 1;
            for child in children {
                result.push((index + offset, child.name.as_str()));
                offset += child.len;
            }
            Some(result)
        })
    }

    pub fn parent_index(&self, target: usize) -> Option<usize> {
        if target == 0 {
            return None; // root has no parent
        }
        Some(self.parent_index_inner(target, 0).expect("unexpected index"))
    }

    fn parent_index_inner(&self, target: usize, self_index: usize) -> Option<usize> {
        let mut child_flat = self_index + 1;
        for child in self.child.as_deref().into_iter().flatten() {
            if target >= child_flat && target < child_flat + child.len {
                if target == child_flat {
                    return Some(self_index);
                }
                return child.parent_index_inner(target, child_flat);
            }
            child_flat += child.len;
        }
        None
    }

    fn formatted_name(&self, is_last: Vec<bool>) -> String {
        prefix(is_last).chain(self.name.chars()).collect()
    }

    pub fn as_windowed_tree_string(&self, selected: Option<usize>) -> Vec<WindowedTreeEntry> {
        let mut result = Vec::new();
        self.windowed_dfs(&mut result, selected, &mut Vec::new(), 0);
        result
    }

    fn find_focus_child(&self, selected: usize, self_flat_index: usize) -> Option<usize> {
        let children = self.child.as_deref()?;
        let mut child_flat = self_flat_index + 1;
        for (i, child) in children.iter().enumerate() {
            if selected >= child_flat && selected < child_flat + child.len {
                return Some(i);
            }
            child_flat += child.len;
        }
        None
    }

    fn windowed_dfs(
        &self,
        result: &mut Vec<WindowedTreeEntry>,
        selected: Option<usize>,
        is_last_stack: &mut Vec<bool>,
        self_flat_index: usize,
    ) {
        result.push(WindowedTreeEntry {
            display: self.formatted_name(is_last_stack.clone()),
            real_index: Some(self_flat_index),
        });

        let Some(children) = &self.child else {
            return;
        };
        let n = children.len();
        if n == 0 {
            return;
        }

        if n <= WINDOW_THRESHOLD {
            let mut child_flat = self_flat_index + 1;
            for (i, child) in children.iter().enumerate() {
                is_last_stack.push(i == n - 1);
                child.windowed_dfs(result, selected, is_last_stack, child_flat);
                is_last_stack.pop();
                child_flat += child.len;
            }
            return;
        }

        let focus = selected.and_then(|sel| self.find_focus_child(sel, self_flat_index));
        let visible = compute_visible_indices(n, focus);

        let mut child_flat = self_flat_index + 1;
        let mut prev_visible: Option<usize> = None;

        for (i, child) in children.iter().enumerate() {
            if visible.contains(&i) {
                if let Some(pv) = prev_visible {
                    if pv + 1 < i {
                        let hidden = i - pv - 1;
                        let ellipsis_prefix: String =
                            prefix(is_last_stack.iter().copied().chain([false]).collect())
                                .collect();
                        result.push(WindowedTreeEntry {
                            display: format!("{}... ({} hidden)", ellipsis_prefix, hidden),
                            real_index: None,
                        });
                    }
                } else if i > 0 {
                    let hidden = i;
                    let ellipsis_prefix: String =
                        prefix(is_last_stack.iter().copied().chain([false]).collect()).collect();
                    result.push(WindowedTreeEntry {
                        display: format!("{}... ({} hidden)", ellipsis_prefix, hidden),
                        real_index: None,
                    });
                }

                is_last_stack.push(i == n - 1);
                child.windowed_dfs(result, selected, is_last_stack, child_flat);
                is_last_stack.pop();
                prev_visible = Some(i);
            }
            child_flat += child.len;
        }
    }
}

pub struct WorkTreeStringIter<'a> {
    stack: Vec<Peekable<Iter<'a, WorkTreeNode>>>,
}

impl<'a> WorkTreeStringIter<'a> {
    fn new(init: Option<&'a [WorkTreeNode]>) -> Self {
        Self {
            stack: if let Some(init) = init {
                vec![init.iter().peekable()]
            } else {
                Vec::new()
            },
        }
    }
}

impl Iterator for WorkTreeStringIter<'_> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        let mut next = None;
        while next.is_none() {
            let next_iter = self.stack.last_mut()?;
            next = next_iter.next();
            if next.is_none() {
                self.stack.pop();
            }
        }

        let next = next?;
        let is_last: Vec<_> = self
            .stack
            .iter_mut()
            .map(|parent| parent.peek().is_none())
            .collect();
        if let Some(child) = &next.child {
            self.stack.push(child.iter().peekable());
        }
        Some(next.formatted_name(is_last))
    }
}

fn compute_visible_indices(n: usize, focus: Option<usize>) -> BTreeSet<usize> {
    let mut visible = BTreeSet::new();
    for i in 0..EDGE_COUNT.min(n) {
        visible.insert(i);
    }
    for i in n.saturating_sub(EDGE_COUNT)..n {
        visible.insert(i);
    }
    if let Some(f) = focus {
        let start = f.saturating_sub(FOCUS_RADIUS);
        let end = (f + FOCUS_RADIUS).min(n - 1);
        for i in start..=end {
            visible.insert(i);
        }
    }
    visible
}

fn prefix(mut is_last: Vec<bool>) -> impl Iterator<Item = char> {
    let last = is_last.pop();

    is_last
        .into_iter()
        .flat_map(|is_last| {
            if is_last {
                [' ', ' ', ' ']
            } else {
                [' ', '│', ' ']
            }
        })
        .chain(match last {
            None => [' '].as_slice().iter().copied(),
            Some(true) => [' ', '└', '─', ' '].as_slice().iter().copied(),
            Some(false) => [' ', '├', '─', ' '].as_slice().iter().copied(),
        })
        .skip(1)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn work_tree_formatting_test() {
        let mut node = WorkTreeNode::new_empty(String::from("root"));
        node.reindex(
            0,
            Index {
                meta: NodeMeta::null(),
                kind: IndexKind::Object(vec![
                    String::from("a"),
                    String::from("b"),
                    String::from("c"),
                    String::from("d"),
                ]),
            },
            true,
        );
        node.reindex(
            1,
            Index {
                meta: NodeMeta::null(),
                kind: IndexKind::Object(vec![String::from("aa"), String::from("ab")]),
            },
            true,
        );
        node.reindex(
            4,
            Index {
                meta: NodeMeta::null(),
                kind: IndexKind::Array(3),
            },
            true,
        );
        node.reindex(
            8,
            Index {
                meta: NodeMeta::null(),
                kind: IndexKind::Array(5),
            },
            true,
        );
        node.close(8);

        assert_eq!(
            node.as_tree_string().collect::<Vec<_>>(),
            vec![
                String::from("root"),
                String::from("├─ a"),
                String::from("│  ├─ aa"),
                String::from("│  └─ ab"),
                String::from("├─ b"),
                String::from("│  ├─ 0"),
                String::from("│  ├─ 1"),
                String::from("│  └─ 2"),
                String::from("├─ c"),
                String::from("└─ d"),
            ]
        );
    }

    #[test]
    fn work_tree_selector_test() {
        let mut node = WorkTreeNode::new_empty(String::from("root"));
        node.reindex(
            0,
            Index {
                meta: NodeMeta::null(),
                kind: IndexKind::Object(vec![
                    String::from("a"),
                    String::from("b"),
                    String::from("c"),
                    String::from("d"),
                ]),
            },
            true,
        );
        node.reindex(
            1,
            Index {
                meta: NodeMeta::null(),
                kind: IndexKind::Object(vec![String::from("aa"), String::from("ab")]),
            },
            true,
        );
        node.reindex(
            4,
            Index {
                meta: NodeMeta::null(),
                kind: IndexKind::Array(3),
            },
            true,
        );

        assert_eq!(node.len(), 10);
        assert_eq!(node.selector(0), Vec::<&str>::new());
        assert_eq!(node.selector(1), vec!["a"]);
        assert_eq!(node.selector(2), vec!["a", "aa"]);
        assert_eq!(node.selector(3), vec!["a", "ab"]);
        assert_eq!(node.selector(4), vec!["b"]);
        assert_eq!(node.selector(5), vec!["b", "0"]);
        assert_eq!(node.selector(8), vec!["c"]);
    }

    #[test]
    fn parent_index_test() {
        let mut node = WorkTreeNode::new_empty(String::from("root"));
        node.reindex(
            0,
            Index {
                meta: NodeMeta::null(),
                kind: IndexKind::Object(vec![
                    String::from("a"),
                    String::from("b"),
                    String::from("c"),
                ]),
            },
            true,
        );
        node.reindex(
            1,
            Index {
                meta: NodeMeta::null(),
                kind: IndexKind::Object(vec![String::from("aa"), String::from("ab")]),
            },
            true,
        );

        // root (0)
        //   a (1)
        //     aa (2)
        //     ab (3)
        //   b (4)
        //   c (5)
        assert_eq!(node.parent_index(0), None);
        assert_eq!(node.parent_index(1), Some(0));
        assert_eq!(node.parent_index(2), Some(1));
        assert_eq!(node.parent_index(3), Some(1));
        assert_eq!(node.parent_index(4), Some(0));
        assert_eq!(node.parent_index(5), Some(0));
    }

    #[test]
    fn direct_children_test() {
        let mut node = WorkTreeNode::new_empty(String::from("root"));
        node.reindex(
            0,
            Index {
                meta: NodeMeta::null(),
                kind: IndexKind::Object(vec![
                    String::from("a"),
                    String::from("b"),
                    String::from("c"),
                ]),
            },
            true,
        );
        node.reindex(
            1,
            Index {
                meta: NodeMeta::null(),
                kind: IndexKind::Object(vec![String::from("aa"), String::from("ab")]),
            },
            true,
        );

        // root (0) -> children: a(1), b(4), c(5)
        let children = node.direct_children(0).unwrap();
        assert_eq!(
            children,
            vec![(1, "a"), (4, "b"), (5, "c")]
        );

        // a (1) -> children: aa(2), ab(3)
        let children = node.direct_children(1).unwrap();
        assert_eq!(children, vec![(2, "aa"), (3, "ab")]);

        // leaf node has no children
        assert_eq!(node.direct_children(2), None);
    }

    fn make_wide_node(n: usize) -> WorkTreeNode {
        let keys: Vec<String> = (0..n).map(|i| format!("k{}", i)).collect();
        let mut node = WorkTreeNode::new_empty(String::from("root"));
        node.reindex(
            0,
            Index {
                meta: NodeMeta::null(),
                kind: IndexKind::Object(keys),
            },
            true,
        );
        node
    }

    #[test]
    fn compute_visible_indices_no_focus_test() {
        let vis = compute_visible_indices(15, None);
        // first 3: 0,1,2  last 3: 12,13,14
        assert_eq!(vis, BTreeSet::from([0, 1, 2, 12, 13, 14]));
    }

    #[test]
    fn compute_visible_indices_focus_middle_test() {
        let vis = compute_visible_indices(15, Some(7));
        // first 3: 0,1,2  focus window: 4..=10  last 3: 12,13,14
        assert_eq!(
            vis,
            BTreeSet::from([0, 1, 2, 4, 5, 6, 7, 8, 9, 10, 12, 13, 14])
        );
    }

    #[test]
    fn compute_visible_indices_focus_start_overlap_test() {
        let vis = compute_visible_indices(15, Some(1));
        // focus window: 0..=4 overlaps with first 3
        assert_eq!(vis, BTreeSet::from([0, 1, 2, 3, 4, 12, 13, 14]));
    }

    #[test]
    fn compute_visible_indices_focus_end_overlap_test() {
        let vis = compute_visible_indices(15, Some(13));
        // focus window: 10..=14 overlaps with last 3
        assert_eq!(vis, BTreeSet::from([0, 1, 2, 10, 11, 12, 13, 14]));
    }

    #[test]
    fn compute_visible_indices_all_visible_test() {
        let vis = compute_visible_indices(6, Some(3));
        // all 6 indices visible since it's small
        assert_eq!(vis, BTreeSet::from([0, 1, 2, 3, 4, 5]));
    }

    #[test]
    fn windowed_tree_no_windowing_for_small_test() {
        let node = make_wide_node(10);
        let windowed = node.as_windowed_tree_string(Some(0));
        let tree_string: Vec<_> = node.as_tree_string().collect();

        // All entries should have real indices and match as_tree_string output
        assert_eq!(windowed.len(), tree_string.len());
        for (w, t) in windowed.iter().zip(tree_string.iter()) {
            assert_eq!(&w.display, t);
            assert!(w.real_index.is_some());
        }
    }

    #[test]
    fn windowed_tree_basic_test() {
        let node = make_wide_node(15);
        // selected=root(0), no focus child
        let entries = node.as_windowed_tree_string(Some(0));
        let displays: Vec<_> = entries.iter().map(|e| e.display.as_str()).collect();

        // root + first 3 + ellipsis + last 3
        assert_eq!(displays[0], "root");
        assert_eq!(displays[1], "├─ k0");
        assert_eq!(displays[2], "├─ k1");
        assert_eq!(displays[3], "├─ k2");
        assert!(displays[4].contains("... (9 hidden)"));
        assert_eq!(displays[5], "├─ k12");
        assert_eq!(displays[6], "├─ k13");
        assert_eq!(displays[7], "└─ k14");
        assert_eq!(entries.len(), 8);

        // Ellipsis entry has no real_index
        assert!(entries[4].real_index.is_none());
    }

    #[test]
    fn windowed_tree_with_focus_test() {
        let node = make_wide_node(15);
        // selected=child index 8 → child_idx=7 (k7)
        // flat index of k7 = 8 (root=0, k0=1, k1=2, ..., k7=8)
        let entries = node.as_windowed_tree_string(Some(8));
        let displays: Vec<_> = entries.iter().map(|e| e.display.as_str()).collect();

        // root + first 3 + gap + focus window (k4..k10) + gap + last 3
        assert_eq!(displays[0], "root");
        assert_eq!(displays[1], "├─ k0");
        assert_eq!(displays[2], "├─ k1");
        assert_eq!(displays[3], "├─ k2");
        assert!(displays[4].contains("... (1 hidden)"));
        assert_eq!(displays[5], "├─ k4");
        assert_eq!(displays[6], "├─ k5");
        assert_eq!(displays[7], "├─ k6");
        assert_eq!(displays[8], "├─ k7");
        assert_eq!(displays[9], "├─ k8");
        assert_eq!(displays[10], "├─ k9");
        assert_eq!(displays[11], "├─ k10");
        assert!(displays[12].contains("... (1 hidden)"));
        assert_eq!(displays[13], "├─ k12");
        assert_eq!(displays[14], "├─ k13");
        assert_eq!(displays[15], "└─ k14");
    }

    #[test]
    fn windowed_tree_nested_test() {
        // Both parent and child have >12 children
        let mut node = make_wide_node(15);
        let child_keys: Vec<String> = (0..15).map(|i| format!("c{}", i)).collect();
        node.reindex(
            1,
            Index {
                meta: NodeMeta::null(),
                kind: IndexKind::Object(child_keys),
            },
            true,
        );

        // Select root → no focus child, child k0's subtree is also windowed
        let entries = node.as_windowed_tree_string(Some(0));
        // k0 is visible and expanded with 15 children, those should also be windowed
        let displays: Vec<_> = entries.iter().map(|e| e.display.as_str()).collect();

        // Root + k0 + k0's windowed children + ellipsis + last 3 of root
        assert_eq!(displays[0], "root");
        assert_eq!(displays[1], "├─ k0");
        // k0's children: first 3
        assert_eq!(displays[2], "│  ├─ c0");
        assert_eq!(displays[3], "│  ├─ c1");
        assert_eq!(displays[4], "│  ├─ c2");
        // k0's ellipsis
        assert!(displays[5].contains("... (9 hidden)"));
        // k0's last 3
        assert_eq!(displays[6], "│  ├─ c12");
        assert_eq!(displays[7], "│  ├─ c13");
        assert_eq!(displays[8], "│  └─ c14");
    }
}
