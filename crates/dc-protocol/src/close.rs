//! Gateway WebSocket close codes and what the client should do about them.

/// Close codes the gateway may send (userdoccers "Gateway Close Event Codes").
pub mod code {
    pub const UNKNOWN_ERROR: u16 = 4000;
    pub const UNKNOWN_OPCODE: u16 = 4001;
    pub const DECODE_ERROR: u16 = 4002;
    pub const NOT_AUTHENTICATED: u16 = 4003;
    pub const AUTHENTICATION_FAILED: u16 = 4004;
    pub const ALREADY_AUTHENTICATED: u16 = 4005;
    pub const SESSION_NO_LONGER_VALID: u16 = 4006;
    pub const INVALID_SEQ: u16 = 4007;
    pub const RATE_LIMITED: u16 = 4008;
    pub const SESSION_TIMED_OUT: u16 = 4009;
    pub const INVALID_SHARD: u16 = 4010;
    pub const SHARDING_REQUIRED: u16 = 4011;
    pub const INVALID_API_VERSION: u16 = 4012;
    pub const INVALID_INTENTS: u16 = 4013;
    pub const DISALLOWED_INTENTS: u16 = 4014;
    pub const TOO_MANY_SESSIONS: u16 = 4015;
    pub const CONNECTION_REQUEST_CANCELED: u16 = 4016;

    /// Normal closure. Sending this invalidates the session.
    pub const NORMAL: u16 = 1000;
    pub const GOING_AWAY: u16 = 1001;
}

/// What to do after the gateway connection ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAction {
    /// Reconnect to `resume_gateway_url` and send Resume.
    Resume,
    /// The session is gone: reconnect to the base URL and Identify again.
    Reidentify,
    /// Do not reconnect; retrying cannot succeed without user action.
    Fatal,
}

/// Maps a close code (or the lack of one) to the next step.
///
/// The docs say to resume when the code "indicates you can reconnect" or when
/// there is no close code at all, but (unlike the official bot docs) do not
/// give a per-code reconnect table.
pub fn close_action(close_code: Option<u16>) -> CloseAction {
    use code::*;
    match close_code {
        None => CloseAction::Resume,
        // A bad token never becomes good by retrying, and repeated auth
        // failures count towards Discord's invalid-request limits.
        Some(AUTHENTICATION_FAILED) => CloseAction::Fatal,
        // Configuration errors: the same Identify would fail the same way.
        Some(INVALID_SHARD | SHARDING_REQUIRED | INVALID_API_VERSION) => CloseAction::Fatal,
        Some(INVALID_INTENTS | DISALLOWED_INTENTS) => CloseAction::Fatal,
        // ASSUMPTION: "too many sessions" and "connection request canceled"
        // are treated as fatal; reconnecting in a loop would only open more
        // sessions. The docs do not say whether these are reconnectable.
        Some(TOO_MANY_SESSIONS | CONNECTION_REQUEST_CANCELED) => CloseAction::Fatal,
        // Explanations in the docs say "start a new session" / session invalid.
        Some(NOT_AUTHENTICATED | SESSION_NO_LONGER_VALID | INVALID_SEQ | SESSION_TIMED_OUT) => {
            CloseAction::Reidentify
        }
        // ASSUMPTION: a server-initiated 1000/1001 invalidates the session
        // (the docs state this for client-initiated closes with those codes).
        Some(NORMAL | GOING_AWAY) => CloseAction::Reidentify,
        // ASSUMPTION: everything else (4000-4002, 4005, 4008, unknown and
        // transport-level codes such as 1006) is resumable, following the
        // official bot documentation's reconnect table.
        Some(_) => CloseAction::Resume,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification() {
        assert_eq!(close_action(None), CloseAction::Resume);
        assert_eq!(close_action(Some(4000)), CloseAction::Resume);
        assert_eq!(close_action(Some(4008)), CloseAction::Resume);
        assert_eq!(close_action(Some(1006)), CloseAction::Resume);
        assert_eq!(close_action(Some(4007)), CloseAction::Reidentify);
        assert_eq!(close_action(Some(4009)), CloseAction::Reidentify);
        assert_eq!(close_action(Some(1000)), CloseAction::Reidentify);
        assert_eq!(close_action(Some(4004)), CloseAction::Fatal);
        assert_eq!(close_action(Some(4012)), CloseAction::Fatal);
        assert_eq!(close_action(Some(4015)), CloseAction::Fatal);
    }
}
