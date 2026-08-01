use crate::account::Role;
use irixmail_core::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Access {
    Administration,
    Mailbox,
}

impl Role {
    pub fn can_access(&self, access: Access) -> bool {
        match self {
            Role::Admin => true,
            Role::User => matches!(access, Access::Mailbox),
        }
    }
}

pub fn authorize(role: Role, access: Access) -> Result<()> {
    if role.can_access(access) {
        Ok(())
    } else {
        Err(Error::forbidden(forbidden_detail(access)))
    }
}

fn forbidden_detail(access: Access) -> &'static str {
    match access {
        Access::Administration => "administrative access requires an administrator role",
        Access::Mailbox => "mailbox access is not permitted for this role",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_admin_reaches_every_surface() {
        assert!(Role::Admin.can_access(Access::Administration));
        assert!(Role::Admin.can_access(Access::Mailbox));
    }

    #[test]
    fn a_user_reaches_only_their_mailbox() {
        assert!(Role::User.can_access(Access::Mailbox));
        assert!(!Role::User.can_access(Access::Administration));
    }

    #[test]
    fn authorize_grants_an_admin_the_administration_surface() {
        assert!(authorize(Role::Admin, Access::Administration).is_ok());
    }

    #[test]
    fn authorize_grants_either_role_a_mailbox() {
        assert!(authorize(Role::Admin, Access::Mailbox).is_ok());
        assert!(authorize(Role::User, Access::Mailbox).is_ok());
    }

    #[test]
    fn authorize_refuses_a_user_the_administration_surface() {
        let err = authorize(Role::User, Access::Administration).expect_err("a user is refused");
        assert!(matches!(err, Error::Forbidden(_)));
    }

    #[test]
    fn a_refused_administration_request_names_the_surface_not_the_role() {
        let err = authorize(Role::User, Access::Administration).expect_err("a user is refused");
        let message = err.to_string();
        assert!(message.contains("administrator"));
        assert!(!message.contains("User"));
    }

    #[test]
    fn the_two_surfaces_are_distinct() {
        assert_ne!(Access::Administration, Access::Mailbox);
    }

    #[test]
    fn the_decision_matches_between_the_boolean_and_the_result_forms() {
        for role in [Role::Admin, Role::User] {
            for access in [Access::Administration, Access::Mailbox] {
                assert_eq!(role.can_access(access), authorize(role, access).is_ok());
            }
        }
    }
}
