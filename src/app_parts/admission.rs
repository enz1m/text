//! TextApp work admission types.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Admission {
    Started,
    // The bounded queue is full; the intent is kept for the next completion.
    Refused,
    NothingToDo,
}

// `InFlight` prevents the close guard from reporting success before durable
// publication settles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SaveDispatch {
    Failed,
    InFlight,
}
