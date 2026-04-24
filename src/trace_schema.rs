//! Shared semantic trace schema for runtime/view latency flows.
//!
//! Keep these names and field spellings aligned with the iOS `BooTrace` helper
//! so Rust tracing output and Apple Logger/signpost output can be correlated.

pub(crate) mod events {
    pub(crate) const REMOTE_CONNECT: &str = "remote.connect";
    pub(crate) const REMOTE_RUNTIME_ACTION: &str = "remote.runtime_action";
    pub(crate) const REMOTE_FOCUS_PANE: &str = "remote.focus_pane";
    pub(crate) const REMOTE_SET_VIEWED_TAB: &str = "remote.set_viewed_tab";
    pub(crate) const REMOTE_RESIZE_SPLIT: &str = "remote.resize_split";
    pub(crate) const REMOTE_INPUT: &str = "remote.input";
    pub(crate) const REMOTE_PANE_UPDATE: &str = "remote.pane_update";
    #[allow(dead_code)]
    pub(crate) const REMOTE_RENDER_APPLY: &str = "remote.render_apply";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeActionKind {
    SetViewedTab,
    FocusPane,
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    AttachView,
    DetachView,
    NewSplit,
    ResizeSplit,
}

impl RuntimeActionKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::SetViewedTab => "set_viewed_tab",
            Self::FocusPane => "focus_pane",
            Self::NewTab => "new_tab",
            Self::CloseTab => "close_tab",
            Self::NextTab => "next_tab",
            Self::PrevTab => "prev_tab",
            Self::AttachView => "attach_view",
            Self::DetachView => "detach_view",
            Self::NewSplit => "new_split",
            Self::ResizeSplit => "resize_split",
        }
    }
}
