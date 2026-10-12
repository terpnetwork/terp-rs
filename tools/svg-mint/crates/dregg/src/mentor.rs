//! Mentor verifies that proof commitments bind to the user's mint action.
//! Opening is not published on chain.

use crate::commit::{bind, verify_action, verify_deposit, ActionOpening, DepositOpening};

#[derive(Clone, Debug)]
pub struct MentorOpening {
    pub deposit: DepositOpening,
    pub action: ActionOpening,
}

pub struct Mentor;

impl Mentor {
    /// Returns true iff openings match the public commitments and the bind.
    pub fn verify_binding(
        c_deposit: &[u8; 32],
        c_action: &[u8; 32],
        bind_tag: &[u8; 32],
        opening: &MentorOpening,
    ) -> bool {
        if !verify_deposit(c_deposit, &opening.deposit) {
            return false;
        }
        if !verify_action(c_action, &opening.action) {
            return false;
        }
        let expected = bind(c_deposit, c_action, &opening.action.session);
        &expected == bind_tag
    }
}
