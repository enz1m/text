//! TextApp document load reason.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoadReason {
    Open,
    Reload,
}
