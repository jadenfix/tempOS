use beater_os_core::{BeaterOsError, SessionStatus};
use thiserror::Error;

/// A lifecycle transition requested against a [`crate::Session`].
///
/// This mirrors the `create/pause/resume/cancel` scope of backlog slice 3. It
/// exists only to give rejected transitions a legible, typed name; it is not a
/// state and carries no data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Pause,
    Resume,
    Cancel,
}

/// Errors returned by the session runtime.
///
/// Every variant is a *rejection*: the runtime is fail-closed, so an illegal
/// transition or an ill-formed grant bind returns one of these and leaves the
/// session (and its journal) unchanged. No variant represents a silent no-op or
/// a partially-applied state change.
///
/// `PartialEq`/`Eq` is intentionally not derived: [`BeaterOsError`] wraps a
/// `serde_json::Error`, which is not comparable. Tests match on variants with
/// `matches!` instead.
#[derive(Debug, Error)]
pub enum SessionError {
    /// The requested transition is not defined from the session's current
    /// status. This is the fail-closed default: any `(transition, status)`
    /// pairing that is not explicitly legal lands here, including double-cancel
    /// and any transition out of a terminal state.
    #[error("illegal session transition: cannot {transition:?} from status {from:?}")]
    IllegalTransition {
        transition: Transition,
        from: SessionStatus,
    },

    /// A grant was bound (or a transition attempted) while the session was not
    /// active. Grants confer no authority on a paused or terminal session
    /// (`final.md` §26 "No ambient authority").
    #[error("session is not active (status {status:?}); grant binding is refused")]
    SessionNotActive { status: SessionStatus },

    /// The grant names a different session than the one it is being bound to.
    #[error("grant {grant_id} targets session {grant_session_id}, not this session {session_id}")]
    GrantSessionMismatch {
        grant_id: String,
        grant_session_id: String,
        session_id: String,
    },

    /// The grant's holder is not this session's principal (its agent identity).
    #[error("grant {grant_id} holder {holder} is not this session's principal {principal}")]
    GrantPrincipalMismatch {
        grant_id: String,
        holder: String,
        principal: String,
    },

    /// The grant is revoked or expired at the moment of binding. Revocation and
    /// expiry fail closed (`final.md` §26 "Revocation").
    #[error("grant {grant_id} is revoked or expired at {now}")]
    GrantInactive { grant_id: String, now: String },

    /// A grant with this id is already bound to the session. Re-binding is
    /// refused (fail-closed) rather than silently deduped, so the journal never
    /// carries a second `CapabilityGranted` for one grant and the audit view
    /// stays unambiguous.
    #[error("grant {grant_id} is already bound to this session")]
    GrantAlreadyBound { grant_id: String },

    /// The underlying append-only journal rejected the record. Propagated from
    /// core so the caller can distinguish a policy rejection from an integrity
    /// fault.
    #[error(transparent)]
    Journal(#[from] BeaterOsError),
}
