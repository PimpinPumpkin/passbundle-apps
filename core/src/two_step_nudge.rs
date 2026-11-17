//! The nudges about two-step login above the vault's list (DESIGN.md in the
//! meta repo, "A nudge for everyone else"): to set it up, for someone with no
//! two-step login at all, and to add a stronger method, for someone whose
//! only second step is email codes. The server decides who sees which
//! (`GET /api/keykeeper/two-step-nudge`), only for people no rule requires
//! it of, and keeps "Not now" for the account, one for both, for 90 days on
//! every device (`POST /api/keykeeper/two-step-nudge/dismiss`). The apps
//! can't set up two-step login themselves: Set it up opens the web vault's
//! two-step settings.
//!
//! Nothing here holds anything up: offline, signed out, or a server without
//! the endpoint (an older one, or someone else's), nothing shows.

use serde::Serialize;
use serde_json::Value;

/// Which nudge the vault shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TwoStepNudge {
    /// No two-step login at all: "Your password is all that protects your
    /// account. ..."
    SetUp,
    /// Email codes and nothing else: "Codes by email are only as safe as
    /// your email account. ..."
    Stronger,
}

/// The nudge in the server's answer, if any. A server from before the
/// second nudge says only `show` (the first nudge); a newer one says which
/// in `nudge`, and keeps `show` for the first only. A nudge this core
/// doesn't know shows nothing.
pub fn from_answer(answer: &Value) -> Option<TwoStepNudge> {
    match answer.get("nudge") {
        None => (answer.get("show").and_then(Value::as_bool) == Some(true))
            .then_some(TwoStepNudge::SetUp),
        Some(nudge) => match nudge.as_str() {
            Some("setUp") => Some(TwoStepNudge::SetUp),
            Some("stronger") => Some(TwoStepNudge::Stronger),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_server_names_the_nudge_and_an_older_one_says_only_show() {
        assert_eq!(
            from_answer(&json!({ "show": true, "nudge": "setUp" })),
            Some(TwoStepNudge::SetUp)
        );
        assert_eq!(
            from_answer(&json!({ "show": false, "nudge": "stronger" })),
            Some(TwoStepNudge::Stronger)
        );
        assert_eq!(from_answer(&json!({ "show": false, "nudge": null })), None);
        // A server from before the second nudge.
        assert_eq!(
            from_answer(&json!({ "show": true })),
            Some(TwoStepNudge::SetUp)
        );
        assert_eq!(from_answer(&json!({ "show": false })), None);
        // Anything else shows nothing: a nudge from a newer server this
        // doesn't know, or an answer that isn't the server's.
        assert_eq!(
            from_answer(&json!({ "show": true, "nudge": "somethingNew" })),
            None
        );
        assert_eq!(from_answer(&json!(null)), None);
        assert_eq!(from_answer(&json!("<html>")), None);
    }

    #[test]
    fn each_nudge_reaches_the_screens_by_name() {
        assert_eq!(json!(TwoStepNudge::SetUp), json!("setUp"));
        assert_eq!(json!(TwoStepNudge::Stronger), json!("stronger"));
        assert_eq!(json!(None::<TwoStepNudge>), json!(null));
    }
}
