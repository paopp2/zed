use collections::BTreeMap;
use git::{
    repository::RepoPath,
    status::TreeDiffStatus,
};
use gpui::{
    App, ClickEvent, Context, EventEmitter, Render, SharedString,
    UniformListScrollHandle, Window, uniform_list,
};
use std::ops::Range;
use theme::ActiveTheme;
use ui::prelude::*;

const TREE_INDENT: f32 = 16.0;

pub enum DiffFileListEvent {
    FileSelected { repo_path: RepoPath },
}

pub struct DiffFileList {
    source_entries: collections::HashMap<RepoPath, TreeDiffStatus>,
    flattened: Vec<DiffFileEntry>,
    expanded_dirs: collections::HashMap<RepoPath, bool>,
    selected_index: Option<usize>,
    scroll_handle: UniformListScrollHandle,
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

impl DiffFileList {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            source_entries: collections::HashMap::default(),
            flattened: Vec::new(),
            expanded_dirs: collections::HashMap::default(),
            selected_index: None,
            scroll_handle: UniformListScrollHandle::new(),
        }
    }

    pub fn update_entries(
        &mut self,
        entries: &collections::HashMap<RepoPath, TreeDiffStatus>,
        cx: &mut Context<Self>,
    ) {
        self.source_entries = entries.clone();
        self.rebuild_flattened();
        cx.notify();
    }

    fn rebuild_flattened(&mut self) {
        let mut sorted_entries: Vec<_> = self.source_entries.iter().collect();
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
            flattened.push(DiffFileEntry::File {
                repo_path: repo_path.clone(),
                name: SharedString::from(file_name),
                depth,
                status: status.clone(),
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

    fn toggle_directory(&mut self, path: &RepoPath, cx: &mut Context<Self>) {
        let expanded = self.expanded_dirs.entry(path.clone()).or_insert(true);
        *expanded = !*expanded;
        self.rebuild_flattened();
        cx.notify();
    }

    fn select_file(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected_index = Some(index);
        if let Some(DiffFileEntry::File { repo_path, .. }) = self.flattened.get(index) {
            cx.emit(DiffFileListEvent::FileSelected {
                repo_path: repo_path.clone(),
            });
        }
        cx.notify();
    }

    fn render_entry(
        &self,
        ix: usize,
        entry: &DiffFileEntry,
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
                    .id(ElementId::Name(format!("diff-dir-{ix}").into()))
                    .w_full()
                    .h(px(28.))
                    .px_2()
                    .pl(px(*depth as f32 * TREE_INDENT + 4.0))
                    .gap_1()
                    .items_center()
                    .hover(|style| style.bg(colors.ghost_element_hover))
                    .when(selected, |this| this.bg(colors.ghost_element_selected))
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
            DiffFileEntry::File { name, depth, status, .. } => {
                let label_color = status_color(status);
                let entry = entry.clone();

                h_flex()
                    .id(ElementId::Name(format!("diff-file-{ix}").into()))
                    .w_full()
                    .h(px(28.))
                    .px_2()
                    .pl(px(*depth as f32 * TREE_INDENT + 4.0))
                    .gap_1()
                    .items_center()
                    .hover(|style| style.bg(colors.ghost_element_hover))
                    .when(selected, |this| this.bg(colors.ghost_element_selected))
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
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        let _ = &entry;
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
                    cx.processor(move |this, range: Range<usize>, _window, cx| {
                        range
                            .map(|ix| {
                                let entry = &entries[ix];
                                this.render_entry(ix, entry, cx)
                            })
                            .collect()
                    }),
                )
                .flex_1()
                .track_scroll(&self.scroll_handle),
            )
    }
}
