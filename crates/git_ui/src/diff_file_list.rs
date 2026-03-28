use collections::BTreeMap;
use git::{
    repository::RepoPath,
    status::{DiffStat, TreeDiffStatus},
};
use gpui::{
    App, ClickEvent, Context, EventEmitter, FocusHandle, Focusable, KeyContext, Render,
    ScrollStrategy, SharedString, UniformListScrollHandle, Window, actions, uniform_list,
};
use menu;
use std::ops::Range;
use theme::ActiveTheme;
use ui::prelude::*;

actions!(
    diff_file_list,
    [CollapseSelectedEntry, ExpandSelectedEntry, FocusEditor]
);

const TREE_INDENT: f32 = 16.0;

pub enum DiffFileListEvent {
    FileSelected { repo_path: RepoPath },
    FocusEditor,
}

pub struct DiffFileList {
    source_entries: collections::HashMap<RepoPath, TreeDiffStatus>,
    stats: Option<collections::HashMap<RepoPath, DiffStat>>,
    flattened: Vec<DiffFileEntry>,
    expanded_dirs: collections::HashMap<RepoPath, bool>,
    selected_index: Option<usize>,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
}

#[derive(Clone)]
enum DiffFileEntry {
    Directory {
        path: RepoPath,
        name: SharedString,
        depth: usize,
        expanded: bool,
    },
    File {
        repo_path: RepoPath,
        name: SharedString,
        depth: usize,
        status: TreeDiffStatus,
        stats: Option<DiffStat>,
    },
}

#[derive(Default)]
struct TreeNode {
    name: SharedString,
    path: Option<RepoPath>,
    children: BTreeMap<SharedString, TreeNode>,
    files: Vec<(RepoPath, TreeDiffStatus)>,
}

impl EventEmitter<DiffFileListEvent> for DiffFileList {}

impl Focusable for DiffFileList {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl DiffFileList {
    fn dispatch_context(&self) -> KeyContext {
        let mut dispatch_context = KeyContext::new_with_defaults();
        dispatch_context.add("DiffFileList");
        dispatch_context.add("menu");
        dispatch_context
    }

    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            source_entries: collections::HashMap::default(),
            stats: None,
            flattened: Vec::new(),
            expanded_dirs: collections::HashMap::default(),
            selected_index: None,
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn update_entries(
        &mut self,
        entries: &collections::HashMap<RepoPath, TreeDiffStatus>,
        stats: Option<&collections::HashMap<RepoPath, DiffStat>>,
        cx: &mut Context<Self>,
    ) {
        if self.source_entries == *entries && self.stats.as_ref() == stats {
            return;
        }

        let selected_path = self.selected_path();

        self.source_entries = entries.clone();
        self.stats = stats.cloned();
        self.rebuild_flattened(entries);

        if let Some(path) = &selected_path {
            self.restore_selection_by_path(path);
        }

        cx.notify();
    }

    fn rebuild_flattened(&mut self, entries: &collections::HashMap<RepoPath, TreeDiffStatus>) {
        let mut sorted_entries: Vec<_> = entries.iter().collect();
        sorted_entries.sort_by(|(a, _), (b, _)| a.cmp(b));

        let mut root = TreeNode::default();
        for (repo_path, status) in sorted_entries {
            let components: Vec<&str> = repo_path.components().collect();
            if components.is_empty() {
                root.files.push((repo_path.clone(), status.clone()));
                continue;
            }

            let mut current = &mut root;
            let mut current_path = String::new();

            for (ix, component) in components.iter().enumerate() {
                if ix == components.len() - 1 {
                    current.files.push((repo_path.clone(), status.clone()));
                } else {
                    if !current_path.is_empty() {
                        current_path.push('/');
                    }
                    current_path.push_str(component);
                    let dir_path = RepoPath::new(&current_path);
                    let component_str = SharedString::from(component.to_string());

                    current = current
                        .children
                        .entry(component_str.clone())
                        .or_insert_with(|| TreeNode {
                            name: component_str,
                            path: dir_path.ok(),
                            ..Default::default()
                        });
                }
            }
        }

        self.flattened = self.flatten_tree(&root, 0);
    }

    fn flatten_tree(&self, node: &TreeNode, depth: usize) -> Vec<DiffFileEntry> {
        let mut flattened = Vec::new();

        for child in node.children.values() {
            let (terminal, name) = Self::compact_directory_chain(child);
            let Some(path) = terminal.path.clone().or_else(|| child.path.clone()) else {
                continue;
            };
            let expanded = *self.expanded_dirs.get(&path).unwrap_or(&true);

            flattened.push(DiffFileEntry::Directory {
                path: path.clone(),
                name,
                depth,
                expanded,
            });

            if expanded {
                let child_entries = self.flatten_tree(terminal, depth + 1);
                flattened.extend(child_entries);
            }
        }

        for (repo_path, status) in &node.files {
            let file_name = repo_path
                .file_name()
                .unwrap_or_default()
                .to_string();
            let stat = self.stats.as_ref().and_then(|s| s.get(repo_path)).copied();
            flattened.push(DiffFileEntry::File {
                repo_path: repo_path.clone(),
                name: SharedString::from(file_name),
                depth,
                status: status.clone(),
                stats: stat,
            });
        }

        flattened
    }

    fn compact_directory_chain(mut node: &TreeNode) -> (&TreeNode, SharedString) {
        let mut parts = vec![node.name.clone()];
        while node.files.is_empty() && node.children.len() == 1 {
            let Some(child) = node.children.values().next() else {
                break;
            };
            if child.path.is_none() {
                break;
            }
            parts.push(child.name.clone());
            node = child;
        }
        let name = parts.join("/");
        (node, SharedString::from(name))
    }

    fn scroll_to_selected_entry(&self) {
        if let Some(index) = self.selected_index {
            self.scroll_handle
                .scroll_to_item(index, ScrollStrategy::Center);
        }
    }

    fn selected_path(&self) -> Option<RepoPath> {
        let ix = self.selected_index?;
        match self.flattened.get(ix) {
            Some(DiffFileEntry::File { repo_path, .. }) => Some(repo_path.clone()),
            Some(DiffFileEntry::Directory { path, .. }) => Some(path.clone()),
            None => None,
        }
    }

    fn restore_selection_by_path(&mut self, path: &RepoPath) {
        self.selected_index = self.flattened.iter().position(|entry| match entry {
            DiffFileEntry::File { repo_path, .. } => repo_path == path,
            DiffFileEntry::Directory { path: p, .. } => p == path,
        });
    }

    fn toggle_directory(&mut self, path: &RepoPath, cx: &mut Context<Self>) {
        let selected_path = self.selected_path();

        let expanded = self.expanded_dirs.entry(path.clone()).or_insert(true);
        *expanded = !*expanded;
        let entries = self.source_entries.clone();
        self.rebuild_flattened(&entries);

        if let Some(path) = &selected_path {
            self.restore_selection_by_path(path);
        }

        cx.notify();
    }

    pub fn select_by_path(&mut self, path: &RepoPath, cx: &mut Context<Self>) {
        let index = self.flattened.iter().position(|entry| {
            matches!(entry, DiffFileEntry::File { repo_path, .. } if repo_path == path)
        });
        if index != self.selected_index {
            self.selected_index = index;
            self.scroll_to_selected_entry();
            cx.notify();
        }
    }

    fn set_selection(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected_index = Some(index);
        self.scroll_to_selected_entry();
        cx.notify();
    }

    fn select_file(&mut self, index: usize, cx: &mut Context<Self>) {
        self.set_selection(index, cx);
        if let Some(DiffFileEntry::File { repo_path, .. }) = self.flattened.get(index) {
            cx.emit(DiffFileListEvent::FileSelected {
                repo_path: repo_path.clone(),
            });
        }
    }

    fn select_next(
        &mut self,
        _: &menu::SelectNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.flattened.is_empty() {
            return;
        }
        let new_index = match self.selected_index {
            Some(ix) => (ix + 1).min(self.flattened.len() - 1),
            None => 0,
        };
        self.set_selection(new_index, cx);
    }

    fn select_previous(
        &mut self,
        _: &menu::SelectPrevious,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.flattened.is_empty() {
            return;
        }
        let new_index = match self.selected_index {
            Some(ix) => ix.saturating_sub(1),
            None => 0,
        };
        self.set_selection(new_index, cx);
    }

    fn select_first(
        &mut self,
        _: &menu::SelectFirst,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.flattened.is_empty() {
            self.set_selection(0, cx);
        }
    }

    fn select_last(
        &mut self,
        _: &menu::SelectLast,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.flattened.is_empty() {
            self.set_selection(self.flattened.len() - 1, cx);
        }
    }

    fn confirm(
        &mut self,
        _: &menu::Confirm,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.selected_index else {
            return;
        };
        match self.flattened.get(index).cloned() {
            Some(DiffFileEntry::File { repo_path, .. }) => {
                cx.emit(DiffFileListEvent::FileSelected { repo_path });
            }
            Some(DiffFileEntry::Directory { path, .. }) => {
                self.toggle_directory(&path, cx);
            }
            None => {}
        }
    }

    fn expand_selected_entry(
        &mut self,
        _: &ExpandSelectedEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.selected_index else {
            return;
        };
        match self.flattened.get(index).cloned() {
            Some(DiffFileEntry::Directory { path, expanded, .. }) => {
                if expanded {
                    self.select_next(&menu::SelectNext, window, cx);
                } else {
                    self.toggle_directory(&path, cx);
                }
            }
            _ => {
                self.select_next(&menu::SelectNext, window, cx);
            }
        }
    }

    fn collapse_selected_entry(
        &mut self,
        _: &CollapseSelectedEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.selected_index else {
            return;
        };
        match self.flattened.get(index).cloned() {
            Some(DiffFileEntry::Directory { path, expanded, .. }) => {
                if expanded {
                    self.toggle_directory(&path, cx);
                } else {
                    self.select_previous(&menu::SelectPrevious, window, cx);
                }
            }
            _ => {
                self.select_previous(&menu::SelectPrevious, window, cx);
            }
        }
    }

    fn focus_editor(
        &mut self,
        _: &FocusEditor,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.emit(DiffFileListEvent::FocusEditor);
    }

    fn render_entry(
        &self,
        ix: usize,
        entry: &DiffFileEntry,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let selected = self.selected_index == Some(ix);
        let colors = cx.theme().colors();

        match entry {
            DiffFileEntry::Directory { path, name, depth, expanded } => {
                let path = path.clone();
                let icon_name = if *expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                };

                h_flex()
                    .id(ElementId::NamedInteger("diff-dir".into(), ix as u64))
                    .w_full()
                    .h(px(28.))
                    .px_2()
                    .pl(px(*depth as f32 * TREE_INDENT + 4.0))
                    .gap_1()
                    .items_center()
                    .hover(|style| style.bg(colors.ghost_element_hover))
                    .when(selected, |this| this.bg(colors.ghost_element_selected))
                    .when(selected && self.focus_handle.is_focused(window), |this| {
                        this.border_1().border_color(colors.panel_focused_border)
                    })
                    .child(
                        Icon::new(icon_name)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new(name.clone())
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.toggle_directory(&path, cx);
                    }))
                    .into_any_element()
            }
            DiffFileEntry::File { name, depth, status, stats, .. } => {
                let label_color = status_color(status);
                let stats = *stats;

                h_flex()
                    .id(ElementId::NamedInteger("diff-file".into(), ix as u64))
                    .w_full()
                    .h(px(28.))
                    .px_2()
                    .pl(px(*depth as f32 * TREE_INDENT + 4.0))
                    .gap_1()
                    .items_center()
                    .hover(|style| style.bg(colors.ghost_element_hover))
                    .when(selected, |this| this.bg(colors.ghost_element_selected))
                    .when(selected && self.focus_handle.is_focused(window), |this| {
                        this.border_1().border_color(colors.panel_focused_border)
                    })
                    .child(
                        Icon::new(status_icon_name(status))
                            .size(IconSize::Small)
                            .color(Color::Custom(status_hsla(status, cx))),
                    )
                    .child(
                        Label::new(name.clone())
                            .size(LabelSize::Small)
                            .color(label_color),
                    )
                    .child(div().flex_grow())
                    .when_some(stats, |this, stat| {
                        this.child(
                            ui::DiffStat::new(
                                ElementId::NamedInteger("stat".into(), ix as u64),
                                stat.added as usize,
                                stat.deleted as usize,
                            )
                            .label_size(LabelSize::XSmall),
                        )
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.select_file(ix, cx);
                    }))
                    .into_any_element()
            }
        }
    }
}

fn status_color(status: &TreeDiffStatus) -> Color {
    match status {
        TreeDiffStatus::Added { .. } => Color::Created,
        TreeDiffStatus::Modified { .. } => Color::Modified,
        TreeDiffStatus::Deleted { .. } => Color::Deleted,
    }
}

fn status_icon_name(status: &TreeDiffStatus) -> IconName {
    match status {
        TreeDiffStatus::Added { .. } => IconName::SquarePlus,
        TreeDiffStatus::Modified { .. } => IconName::SquareDot,
        TreeDiffStatus::Deleted { .. } => IconName::SquareMinus,
    }
}

fn status_hsla(status: &TreeDiffStatus, cx: &App) -> gpui::Hsla {
    let colors = cx.theme().colors();
    match status {
        TreeDiffStatus::Added { .. } => colors.version_control_added,
        TreeDiffStatus::Modified { .. } => colors.version_control_modified,
        TreeDiffStatus::Deleted { .. } => colors.version_control_deleted,
    }
}

impl Render for DiffFileList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entry_count = self.flattened.len();
        let entries = self.flattened.clone();

        v_flex()
            .key_context(self.dispatch_context())
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::select_first))
            .on_action(cx.listener(Self::select_last))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::expand_selected_entry))
            .on_action(cx.listener(Self::collapse_selected_entry))
            .on_action(cx.listener(Self::focus_editor))
            .size_full()
            .overflow_hidden()
            .child(
                h_flex()
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(
                        Label::new("Changed Files")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .child(
                uniform_list(
                    "diff-file-list",
                    entry_count,
                    cx.processor(move |this, range: Range<usize>, window, cx| {
                        range
                            .map(|ix| {
                                let entry = &entries[ix];
                                this.render_entry(ix, entry, window, cx)
                            })
                            .collect()
                    }),
                )
                .flex_1()
                .track_scroll(&self.scroll_handle),
            )
    }
}
