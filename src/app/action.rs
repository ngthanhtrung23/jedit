use std::collections::VecDeque;

use crate::container::node::Node;

use super::math::Op;

#[derive(Debug, Clone, Copy)]
#[cfg_attr(test, derive(PartialEq))]
pub enum PreviewNavigationAction {
    Up(u16),
    Down(u16),
    Left,
    Right,
}

impl From<PreviewNavigationAction> for Action {
    fn from(value: PreviewNavigationAction) -> Self {
        NavigationAction::PreviewNavigation(value).into()
    }
}

impl From<PreviewNavigationAction> for WorkSpaceAction {
    fn from(value: PreviewNavigationAction) -> Self {
        NavigationAction::PreviewNavigation(value).into()
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(test, derive(PartialEq))]
pub enum SearchAction {
    Start,
    Input(char),
    Backspace,
    Confirm,
    Cancel,
    Next,
    Previous,
}

impl From<SearchAction> for Action {
    fn from(value: SearchAction) -> Self {
        NavigationAction::Search(value).into()
    }
}

impl From<SearchAction> for WorkSpaceAction {
    fn from(value: SearchAction) -> Self {
        NavigationAction::Search(value).into()
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq, Clone, Copy))]
pub enum NavigationAction {
    Up(usize),
    Down(usize),
    Top,
    Bottom,
    Expand,
    Close,
    CloseOrCloseParent,
    TogglePreview,
    PreviewNavigation(PreviewNavigationAction),
    PreviewWindowResize(Op),
    Search(SearchAction),
}

impl From<NavigationAction> for Action {
    fn from(value: NavigationAction) -> Self {
        WorkSpaceAction::from(value).into()
    }
}

impl From<NavigationAction> for WorkSpaceAction {
    fn from(value: NavigationAction) -> Self {
        WorkSpaceAction::Navigation(value)
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq, Clone, Copy))]
pub enum ConfirmAction<T, C = bool> {
    Request(T),
    Confirm(C),
}

impl<T, C> ConfirmAction<T, C> {
    pub(crate) fn action_confirmer<R: Into<Action>>(
        f: impl Fn(ConfirmAction<T, C>) -> R,
    ) -> impl Fn(C) -> Action {
        move |b| f(ConfirmAction::Confirm(b)).into()
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq, Clone))]
pub(crate) enum WorkSpaceAction {
    Navigation(NavigationAction),
    Edit,
    EditError(ConfirmAction<String>),
    Save(ConfirmAction<()>),
    SaveDone,
    ErrorConfirmed,
    Load { node: Node, is_edit: bool },
    Rename(ConfirmAction<(), Option<String>>),
    Delete(ConfirmAction<()>),
    Add(ConfirmAction<(), Option<String>>),
}

impl From<WorkSpaceAction> for Action {
    fn from(value: WorkSpaceAction) -> Self {
        Self::Workspace(value)
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub enum EditJobAction {
    Init,
    Open,
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub enum JobAction {
    Edit(EditJobAction),
    Save,
}

impl From<JobAction> for Action {
    fn from(value: JobAction) -> Self {
        Self::ExecuteJob(value)
    }
}

#[must_use]
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) enum Action {
    Exit(ConfirmAction<()>),
    Workspace(WorkSpaceAction),
    ExecuteJob(JobAction),
}

pub struct Actions(VecDeque<Action>);

impl Actions {
    pub fn new() -> Self {
        Self(VecDeque::new())
    }

    pub(crate) fn push(&mut self, action: Action) {
        self.0.push_back(action);
    }

    pub(crate) fn next(&mut self) -> Option<Action> {
        self.0.pop_front()
    }

    #[cfg(test)]
    pub(crate) fn into_vec(self) -> Vec<Action> {
        self.0.into_iter().collect()
    }
}

impl Default for Actions {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn actions_in_order_test() {
        let mut actions = Actions::new();
        actions.push(Action::Workspace(WorkSpaceAction::Edit));
        actions.push(Action::Exit(ConfirmAction::Request(())));
        assert_eq!(
            actions.next(),
            Some(Action::Workspace(WorkSpaceAction::Edit))
        );
        assert_eq!(
            actions.next(),
            Some(Action::Exit(ConfirmAction::Request(())))
        );
        assert_eq!(actions.next(), None);
    }
}
