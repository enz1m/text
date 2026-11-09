use super::*;

use enzui_core::app_data::{AppDataResolver, AppId};
use enzui_core::blocking_task::{BlockingTask, BlockingTaskDispatcher, BlockingTaskError};
use enzui_core::quarantine::{QuarantineOutcome, quarantine_sibling_expected};

// Bounded summary of an over-bound projection: counts, aggregate decoded
// path bytes, and the refused bound. Retained in place of the projection so
// no tab or workspace content survives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PublishOverBoundSummary {
    pub tabs: usize,
    pub workspaces: usize,
    pub aggregate_path_bytes: u64,
    pub reason: SessionRefusal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionDebtKind {
    Busy,
    Superseded,
    Refused(SessionRefusal),
    FutureVersion,
    Replaced,
    CustodyLost,
    DurabilityUncertain,
    // The live projection exceeded a publication bound. No retryable payload
    // is retained; a fresh bounded projection must be built before
    // republishing.
    PublishOverBound(PublishOverBoundSummary),
    // The checked session-projection revision counter cannot advance. This
    // is terminal: publication is refused, and no fresh projection can
    // supersede a prior refusal.
    PublishRevisionExhausted,
}

impl SessionDebtKind {}

#[derive(Clone, Debug)]
pub(crate) struct SessionDebt {
    request: SessionRequest,
    kind: SessionDebtKind,
    expected: Option<ExpectedSourceIdentity>,
    expected_absent: bool,
}

impl SessionDebt {
    pub(super) fn new(
        request: SessionRequest,
        kind: SessionDebtKind,
        expected: Option<ExpectedSourceIdentity>,
        expected_absent: bool,
    ) -> Self {
        Self {
            request,
            kind,
            expected,
            expected_absent,
        }
    }

    #[must_use]
    pub(crate) const fn kind(&self) -> SessionDebtKind {
        self.kind
    }

    #[must_use]
    pub(crate) fn request(&self) -> &SessionRequest {
        &self.request
    }

    #[must_use]
    pub(crate) const fn expected_identity(&self) -> Option<ExpectedSourceIdentity> {
        self.expected
    }

    #[must_use]
    pub(crate) const fn expected_absent(&self) -> bool {
        self.expected_absent
    }

    #[must_use]
    pub(crate) const fn can_retry(&self) -> bool {
        !matches!(
            self.kind,
            SessionDebtKind::CustodyLost | SessionDebtKind::PublishRevisionExhausted,
        )
    }

    #[must_use]
    pub(crate) fn location(&self) -> &Path {
        self.request.path()
    }

    #[must_use]
    pub(crate) fn from_worker_error(request: SessionRequest, error: BlockingTaskError) -> Self {
        Self::new(request, worker_debt_kind(error), None, false)
    }

    #[must_use]
    pub(crate) fn with_worker_error(&self, error: BlockingTaskError) -> Self {
        Self::new(
            self.request.clone(),
            worker_debt_kind(error),
            self.expected,
            self.expected_absent,
        )
    }

    #[must_use]
    pub(crate) fn superseded(&self) -> Self {
        Self::new(
            self.request.clone(),
            SessionDebtKind::Superseded,
            self.expected,
            self.expected_absent,
        )
    }

    #[must_use]
    pub(crate) fn superseded_request(request: SessionRequest) -> Self {
        Self::new(request, SessionDebtKind::Superseded, None, false)
    }

    // Debt carrying only the bounded summary. Use when the refusal was
    // preflighted without opening the destination, so no expected identity
    // is bound.
    #[must_use]
    pub(crate) fn publish_over_bound(
        request: SessionRequest,
        summary: PublishOverBoundSummary,
    ) -> Self {
        Self::new(
            request,
            SessionDebtKind::PublishOverBound(summary),
            None,
            false,
        )
    }

    // Terminal debt: the projection-revision counter is exhausted and cannot
    // advance without wrapping. Publication is refused and no retry may
    // clear it.
    #[must_use]
    pub(crate) fn publish_revision_exhausted(request: SessionRequest) -> Self {
        Self::new(
            request,
            SessionDebtKind::PublishRevisionExhausted,
            None,
            false,
        )
    }
}

fn worker_debt_kind(error: BlockingTaskError) -> SessionDebtKind {
    match error {
        BlockingTaskError::Busy => SessionDebtKind::Busy,
        BlockingTaskError::Cancelled
        | BlockingTaskError::Shutdown
        | BlockingTaskError::Panicked => SessionDebtKind::Refused(SessionRefusal::OpenRefused),
        _ => SessionDebtKind::Refused(SessionRefusal::OpenRefused),
    }
}

#[derive(Clone, Debug)]
enum SessionParent {
    #[cfg(test)]
    Existing,
    PrivateAppData(AppDataResolver),
}

// One bounded request for session root establishment, admission, and decoding.
#[derive(Clone, Debug)]
pub(crate) struct SessionRequest {
    path: PathBuf,
    home: Option<PathBuf>,
    parent: SessionParent,
    limits: SessionLimits,
}

impl SessionRequest {
    #[must_use]
    pub(crate) fn for_roots(
        data_home: Option<&Path>,
        home: Option<&Path>,
        runtime_fallback: Option<&Path>,
    ) -> Option<Self> {
        let app = AppId::new("text-app").ok()?;
        let primary = AppDataResolver::new(data_home, home);
        let resolver = if primary.resolve(&app).is_ok() {
            primary
        } else {
            AppDataResolver::new(runtime_fallback, None::<&Path>)
        };
        let path = resolver.resolve(&app).ok()?.join("session.list");
        Some(Self {
            path,
            home: home.map(Path::to_path_buf),
            parent: SessionParent::PrivateAppData(resolver),
            limits: SessionLimits::PRODUCTION,
        })
    }

    #[must_use]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    // Creates and verifies the private directory the request's file lives
    // in, as the restore worker does before opening it. `false` when the
    // directory cannot be established.
    pub(crate) fn establish_directory(&self) -> bool {
        self.establish_parent().is_ok()
    }

    pub(crate) fn dispatch(
        self,
        worker: &BlockingTaskDispatcher,
        generation: u64,
    ) -> BlockingTask<SessionRestore> {
        worker.submit(generation, move |_| self.execute())
    }

    pub(crate) fn dispatch_retry(
        self,
        worker: &BlockingTaskDispatcher,
        generation: u64,
        expected: Option<ExpectedSourceIdentity>,
        expected_absent: bool,
    ) -> BlockingTask<SessionRestore> {
        worker.submit(generation, move |_| {
            self.execute_expected(expected, expected_absent)
        })
    }

    pub(crate) fn dispatch_retire(
        self,
        worker: &BlockingTaskDispatcher,
        generation: u64,
        expected: ExpectedSourceIdentity,
    ) -> BlockingTask<SessionRestore> {
        worker.submit(generation, move |_| self.retire(expected))
    }

    pub(super) fn execute(self) -> SessionRestore {
        if self.establish_parent().is_err() {
            return self.refused(SessionRefusal::OpenRefused, None);
        }
        let observed = match observe_path(&self.path) {
            Ok(observed) => observed,
            Err(refusal) => return self.refused(refusal, None),
        };
        match StateAuthority::open_existing_tightening(&self.path) {
            Ok(authority) => {
                let observed = reobserve_after_tightening(&self.path, observed);
                self.read_existing(authority, observed)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && observed.is_none() => {
                self.read_missing()
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.refused(SessionRefusal::Replaced, None)
            }
            Err(_) => self.refused(SessionRefusal::OpenRefused, None),
        }
    }

    fn execute_expected(
        self,
        expected: Option<ExpectedSourceIdentity>,
        expected_absent: bool,
    ) -> SessionRestore {
        if self.establish_parent().is_err() {
            return self.refused(SessionRefusal::OpenRefused, None);
        }
        if expected_absent {
            return self.read_missing();
        }
        let Some(expected) = expected else {
            return self.execute();
        };
        let observed = match observe_path(&self.path) {
            Ok(observed) => observed,
            Err(refusal) => return self.refused(refusal, None),
        };
        match StateAuthority::open_existing_tightening(&self.path) {
            Ok(authority)
                if authority.identity().is_some_and(|identity| {
                    identity.device() == expected.device() && identity.inode() == expected.inode()
                }) =>
            {
                let observed = reobserve_after_tightening(&self.path, observed);
                self.read_existing(authority, observed)
            }
            _ => self.refused_expected(
                SessionRefusal::Replaced,
                expected,
                SessionDebtKind::Replaced,
            ),
        }
    }

    fn establish_parent(&self) -> Result<(), ()> {
        match &self.parent {
            #[cfg(test)]
            SessionParent::Existing => Ok(()),
            SessionParent::PrivateAppData(resolver) => {
                let app = AppId::new("text-app").map_err(|_| ())?;
                let dir = resolver.ensure_tightening(&app).map_err(|_| ())?;
                let resolved = dir.join("session.list").map_err(|_| ())?;
                (resolved == self.path).then_some(()).ok_or(())
            }
        }
    }

    fn read_missing(self) -> SessionRestore {
        match StateAuthority::create_new(&self.path) {
            Ok(authority) => {
                seam_after_read();
                match std::fs::symlink_metadata(&self.path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        let snapshot = SessionSnapshot::empty();
                        SessionRestore::new(RestorePayload {
                            request: self,
                            snapshot,
                            origin: Origin::Missing,
                            authority: Some(authority),
                            admission: None,
                            expected: None,
                            expected_absent: true,
                            debt_kind: None,
                        })
                    }
                    _ => self.refused_absent(SessionRefusal::Replaced),
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                self.refused_absent(SessionRefusal::Replaced)
            }
            Err(_) => self.refused(SessionRefusal::OpenRefused, None),
        }
    }

    fn read_existing(
        self,
        authority: StateAuthority,
        observed: Option<FileAdmission>,
    ) -> SessionRestore {
        let expected = authority.identity();
        let (text, admission) = match read_admitted(&self.path, self.limits.max_file_bytes) {
            Ok(read) => read,
            Err((refusal, Some(admission)))
                if observed == Some(admission)
                    && expected.is_some_and(|identity| {
                        identity.device() == admission.identity.dev
                            && identity.inode() == admission.identity.ino
                    }) =>
            {
                return self.refused_existing(refusal, Some(admission), authority);
            }
            Err((refusal, None)) => return self.refused_existing(refusal, None, authority),
            Err((_, Some(_))) => {
                return self.refused_existing(SessionRefusal::Replaced, None, authority);
            }
        };
        if observed.is_none() {
            return self.refused_existing(SessionRefusal::Replaced, None, authority);
        }
        if observed != Some(admission)
            || !expected.is_some_and(|identity| {
                identity.device() == admission.identity.dev
                    && identity.inode() == admission.identity.ino
            })
        {
            return self.refused_existing(SessionRefusal::Replaced, None, authority);
        }
        let origin_and_snapshot = match parse_snapshot(&text, self.home.as_deref(), self.limits) {
            Ok(snapshot) => (Origin::Restored, snapshot),
            Err(LoadError::Future(version)) => (
                Origin::ReadOnlyFutureVersion(version),
                SessionSnapshot::empty(),
            ),
            Err(LoadError::Refused(refusal)) => {
                (Origin::Refused(refusal), SessionSnapshot::empty())
            }
            #[cfg(test)]
            Err(LoadError::Missing) => (
                Origin::Refused(SessionRefusal::ReadFailed),
                SessionSnapshot::empty(),
            ),
        };
        seam_after_read();
        if !path_still_matches(&self.path, admission) {
            return self.refused_existing(SessionRefusal::Replaced, Some(admission), authority);
        }
        SessionRestore::new(RestorePayload {
            request: self,
            snapshot: origin_and_snapshot.1,
            origin: origin_and_snapshot.0,
            authority: Some(authority),
            admission: Some(admission),
            expected: None,
            expected_absent: false,
            debt_kind: None,
        })
    }

    fn retire(self, expected: ExpectedSourceIdentity) -> SessionRestore {
        match quarantine_sibling_expected(&self.path, expected) {
            QuarantineOutcome::Moved { custody } if custody.source_paths().next().is_some() => {
                match StateAuthority::create_new(&self.path) {
                    Ok(authority) => SessionRestore::new(RestorePayload {
                        request: self,
                        snapshot: SessionSnapshot::empty(),
                        origin: Origin::Missing,
                        authority: Some(authority),
                        admission: None,
                        expected: None,
                        expected_absent: true,
                        debt_kind: None,
                    }),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => self
                        .refused_expected(
                            SessionRefusal::Replaced,
                            expected,
                            SessionDebtKind::Replaced,
                        ),
                    Err(_) => self.refused_expected(
                        SessionRefusal::OpenRefused,
                        expected,
                        SessionDebtKind::Refused(SessionRefusal::OpenRefused),
                    ),
                }
            }
            QuarantineOutcome::Moved { .. } => self.refused_expected(
                SessionRefusal::OpenRefused,
                expected,
                SessionDebtKind::Refused(SessionRefusal::OpenRefused),
            ),
            QuarantineOutcome::SourceMismatch { .. } | QuarantineOutcome::SourceReplaced { .. } => {
                self.refused_expected(
                    SessionRefusal::Replaced,
                    expected,
                    SessionDebtKind::Replaced,
                )
            }
            QuarantineOutcome::DurabilityUncertain { .. }
            | QuarantineOutcome::PartialCustody { .. } => self.refused_expected(
                SessionRefusal::OpenRefused,
                expected,
                SessionDebtKind::DurabilityUncertain,
            ),
            QuarantineOutcome::CollisionExhausted { .. } | QuarantineOutcome::Refused { .. } => {
                self.refused_expected(
                    SessionRefusal::OpenRefused,
                    expected,
                    SessionDebtKind::Refused(SessionRefusal::OpenRefused),
                )
            }
            _ => self.refused_expected(
                SessionRefusal::OpenRefused,
                expected,
                SessionDebtKind::Refused(SessionRefusal::OpenRefused),
            ),
        }
    }

    fn refused(self, refusal: SessionRefusal, admission: Option<FileAdmission>) -> SessionRestore {
        SessionRestore::new(RestorePayload {
            request: self,
            snapshot: SessionSnapshot::empty(),
            origin: Origin::Refused(refusal),
            authority: None,
            admission,
            expected: None,
            expected_absent: false,
            debt_kind: None,
        })
    }

    fn refused_existing(
        self,
        refusal: SessionRefusal,
        admission: Option<FileAdmission>,
        authority: StateAuthority,
    ) -> SessionRestore {
        SessionRestore::new(RestorePayload {
            request: self,
            snapshot: SessionSnapshot::empty(),
            origin: Origin::Refused(refusal),
            authority: Some(authority),
            admission,
            expected: None,
            expected_absent: false,
            debt_kind: None,
        })
    }

    fn refused_absent(self, refusal: SessionRefusal) -> SessionRestore {
        SessionRestore::new(RestorePayload {
            request: self,
            snapshot: SessionSnapshot::empty(),
            origin: Origin::Refused(refusal),
            authority: None,
            admission: None,
            expected: None,
            expected_absent: true,
            debt_kind: Some(SessionDebtKind::Replaced),
        })
    }

    fn refused_expected(
        self,
        refusal: SessionRefusal,
        expected: ExpectedSourceIdentity,
        debt_kind: SessionDebtKind,
    ) -> SessionRestore {
        SessionRestore::new(RestorePayload {
            request: self,
            snapshot: SessionSnapshot::empty(),
            origin: Origin::Refused(refusal),
            authority: None,
            admission: None,
            expected: Some(expected),
            expected_absent: false,
            debt_kind: Some(debt_kind),
        })
    }
}

fn reobserve_after_tightening(path: &Path, before: Option<FileAdmission>) -> Option<FileAdmission> {
    let before = before?;
    match observe_path(path) {
        Ok(Some(after)) if after.identity == before.identity => Some(after),
        _ => None,
    }
}

fn observe_path(path: &Path) -> Result<Option<FileAdmission>, SessionRefusal> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(Some(FileAdmission::of(&metadata))),
        Ok(metadata) if metadata.file_type().is_symlink() => Err(SessionRefusal::OpenRefused),
        Ok(_) => Err(SessionRefusal::NotRegularFile),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(SessionRefusal::OpenRefused),
    }
}

fn read_admitted(
    path: &Path,
    max_bytes: u64,
) -> Result<(String, FileAdmission), (SessionRefusal, Option<FileAdmission>)> {
    let file = match open_regular_nofollow(path) {
        Ok(file) => file,
        Err(OpenRefusal::Missing) => return Err((SessionRefusal::Replaced, None)),
        Err(OpenRefusal::NotRegular) => return Err((SessionRefusal::NotRegularFile, None)),
        Err(OpenRefusal::Denied) => return Err((SessionRefusal::OpenRefused, None)),
    };
    let metadata = file
        .metadata()
        .map_err(|_| (SessionRefusal::ReadFailed, None))?;
    if !metadata.is_file() {
        return Err((SessionRefusal::NotRegularFile, None));
    }
    let admission = FileAdmission::of(&metadata);
    if admission.len > max_bytes {
        return Err((SessionRefusal::TooLarge, Some(admission)));
    }
    seam_after_metadata();
    let mut text = String::new();
    let mut bounded = (&file).take(max_bytes.saturating_add(1));
    if bounded.read_to_string(&mut text).is_err() {
        return Err((SessionRefusal::ReadFailed, Some(admission)));
    }
    if text.len() as u64 > max_bytes {
        return Err((SessionRefusal::TooLarge, Some(admission)));
    }
    let settled = file
        .metadata()
        .map_err(|_| (SessionRefusal::ReadFailed, Some(admission)))?;
    if FileAdmission::of(&settled) != admission || text.len() as u64 != admission.len {
        return Err((SessionRefusal::Unstable, Some(admission)));
    }
    Ok((text, admission))
}

#[cfg(test)]
impl SessionRequest {
    pub(super) fn with_limits(mut self, limits: SessionLimits) -> Self {
        self.limits = limits;
        self
    }

    #[must_use]
    pub(crate) fn for_path(path: PathBuf, home: Option<&Path>) -> Self {
        Self {
            path,
            home: home.map(Path::to_path_buf),
            parent: SessionParent::Existing,
            limits: SessionLimits::PRODUCTION,
        }
    }
}
