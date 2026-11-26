//! What a split tab holds: shells, plus files, Docker and logs views merged in from their tabs.

use super::*;

impl PaneView {
    pub(super) fn shell(&self) -> Option<&Pane> {
        match self {
            PaneView::Shell(p) => Some(p),
            PaneView::View(_) => None,
        }
    }

    pub(super) fn shell_mut(&mut self) -> Option<&mut Pane> {
        match self {
            PaneView::Shell(p) => Some(p),
            PaneView::View(_) => None,
        }
    }

    pub(super) fn tab(&self) -> Option<&Tab> {
        match self {
            PaneView::Shell(_) => None,
            PaneView::View(t) => Some(t),
        }
    }

    pub(super) fn account(&self) -> Option<&str> {
        match self {
            PaneView::Shell(p) => Some(&p.account),
            PaneView::View(t) => t.account.as_deref(),
        }
    }

    pub(super) fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self {
            PaneView::Shell(p) => p.view.focus_handle(cx),
            PaneView::View(t) => match &t.kind {
                TabKind::Sftp(v) => v.focus_handle(cx),
                TabKind::Logs(v) => v.focus_handle(cx),
                _ => t.focus.clone(),
            },
        }
    }

    pub(super) fn title(&self, cx: &App) -> String {
        match self {
            PaneView::Shell(p) => p
                .view
                .read(cx)
                .title()
                .filter(|t| !t.is_empty())
                .map_or_else(|| "shell".into(), |t| privacy::redact(&t).into_owned()),
            PaneView::View(t) => privacy::redact(t.name.as_ref().unwrap_or(&t.label)).into_owned(),
        }
    }

    pub(super) fn element(&self) -> AnyElement {
        match self {
            PaneView::Shell(p) => p.view.clone().into_any_element(),
            PaneView::View(t) => match &t.kind {
                TabKind::Terminal(_) => div().into_any_element(),
                TabKind::Docker(v) => v.clone().into_any_element(),
                TabKind::Sftp(v) => v.clone().into_any_element(),
                TabKind::Logs(v) => v.clone().into_any_element(),
            },
        }
    }
}

impl TermTab {
    pub(super) fn focused_shell(&self) -> Option<&Pane> {
        self.panes
            .get(self.focused)
            .and_then(PaneView::shell)
            .or_else(|| self.panes.iter().find_map(PaneView::shell))
    }

    pub(super) fn new(panes: Vec<PaneView>) -> Self {
        Self {
            panes,
            focused: 0,
            layout: vec![vec![0]],
            cols: vec![Spring::new(1.0)],
            rows: vec![vec![Spring::new(1.0)]],
            ticked: None,
        }
    }
}

impl Tab {
    pub(super) fn pane_count(&self) -> usize {
        match &self.kind {
            TabKind::Terminal(t) => t.panes.len(),
            _ => 1,
        }
    }

    pub(super) fn into_panes(self) -> Vec<PaneView> {
        match self.kind {
            TabKind::Terminal(t) => t.panes,
            _ => vec![PaneView::View(Box::new(self))],
        }
    }
}
