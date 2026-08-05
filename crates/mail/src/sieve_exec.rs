use irixmail_sieve::{execute, Envelope, Limits, Script};

use crate::message_data::Keyword;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SieveOutcome {
    pub keep: bool,
    pub discarded: bool,
    pub file_into: Vec<String>,
    pub redirects: Vec<String>,
    pub flags: Vec<Keyword>,
}

pub fn execute_sieve(
    script: &Script,
    raw: &[u8],
    mail_from: &str,
    recipient: &str,
) -> SieveOutcome {
    let outcome = execute(
        script,
        raw,
        Envelope {
            from: mail_from,
            to: recipient,
        },
        &Limits::default(),
    );
    let mut flags = Vec::new();
    for flag in &outcome.flags {
        let keyword = Keyword::from_imap(flag);
        if !flags.contains(&keyword) {
            flags.push(keyword);
        }
    }
    SieveOutcome {
        keep: outcome.keep,
        discarded: outcome.discarded,
        file_into: outcome.file_into,
        redirects: outcome.redirects,
        flags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sieve_compile::{
        compile_rules, Action, Comparator, Condition, Field, Rule, RuleSet,
    };

    const MESSAGE: &[u8] = concat!(
        "From: newsletter@example.com\r\n",
        "To: me@example.org\r\n",
        "Subject: Weekly deals\r\n",
        "\r\n",
        "Hello!\r\n"
    )
    .as_bytes();

    fn run_rules(rules: Vec<Rule>) -> SieveOutcome {
        let compiled = compile_rules(&RuleSet { rules }).unwrap();
        execute_sieve(
            &compiled.script,
            MESSAGE,
            "newsletter@example.com",
            "me@example.org",
        )
    }

    fn rule(conditions: Vec<Condition>, actions: Vec<Action>) -> Rule {
        Rule {
            name: "test".to_string(),
            enabled: true,
            match_type: Default::default(),
            conditions,
            actions,
        }
    }

    #[test]
    fn an_empty_script_keeps_the_message_in_the_inbox() {
        let outcome = run_rules(Vec::new());
        assert!(outcome.keep);
        assert!(!outcome.discarded);
        assert!(outcome.file_into.is_empty());
        assert!(outcome.redirects.is_empty());
        assert!(outcome.flags.is_empty());
    }

    #[test]
    fn a_matching_rule_files_into_a_mailbox_and_overrides_the_keep() {
        let outcome = run_rules(vec![rule(
            vec![Condition {
                field: Field::From,
                comparator: Comparator::Contains,
                value: "newsletter@example.com".to_string(),
            }],
            vec![Action::FileInto {
                mailbox: "Newsletters".to_string(),
            }],
        )]);
        assert!(!outcome.keep);
        assert_eq!(outcome.file_into, vec!["Newsletters"]);
    }

    #[test]
    fn a_non_matching_rule_leaves_the_default_keep() {
        let outcome = run_rules(vec![rule(
            vec![Condition {
                field: Field::From,
                comparator: Comparator::Contains,
                value: "someone-else@example.com".to_string(),
            }],
            vec![Action::FileInto {
                mailbox: "Newsletters".to_string(),
            }],
        )]);
        assert!(outcome.keep);
        assert!(outcome.file_into.is_empty());
    }

    #[test]
    fn discard_drops_the_message_without_keeping_it() {
        let outcome = run_rules(vec![rule(Vec::new(), vec![Action::Discard])]);
        assert!(outcome.discarded);
        assert!(!outcome.keep);
        assert!(outcome.file_into.is_empty());
    }

    #[test]
    fn add_flag_collects_the_keyword_on_the_kept_message() {
        let outcome = run_rules(vec![rule(
            Vec::new(),
            vec![Action::AddFlag {
                flag: "\\Flagged".to_string(),
            }],
        )]);
        assert!(outcome.keep);
        assert!(outcome.flags.contains(&Keyword::Flagged));
    }

    #[test]
    fn add_flag_then_file_into_carries_the_flag_to_the_mailbox() {
        let outcome = run_rules(vec![rule(
            Vec::new(),
            vec![
                Action::AddFlag {
                    flag: "Important".to_string(),
                },
                Action::FileInto {
                    mailbox: "Priority".to_string(),
                },
            ],
        )]);
        assert_eq!(outcome.file_into, vec!["Priority"]);
        assert!(outcome
            .flags
            .contains(&Keyword::Custom("Important".to_string())));
    }

    #[test]
    fn redirect_records_the_forward_address() {
        let outcome = run_rules(vec![rule(
            Vec::new(),
            vec![Action::Redirect {
                address: "elsewhere@example.net".to_string(),
            }],
        )]);
        assert_eq!(outcome.redirects, vec!["elsewhere@example.net"]);
        assert!(outcome.keep);
    }

    #[test]
    fn stop_prevents_a_later_rule_from_running() {
        let outcome = run_rules(vec![
            rule(
                Vec::new(),
                vec![
                    Action::FileInto {
                        mailbox: "First".to_string(),
                    },
                    Action::Stop,
                ],
            ),
            rule(
                Vec::new(),
                vec![Action::FileInto {
                    mailbox: "Second".to_string(),
                }],
            ),
        ]);
        assert_eq!(outcome.file_into, vec!["First"]);
    }

    #[test]
    fn system_flags_map_to_keywords_case_insensitively() {
        let outcome = run_rules(vec![rule(
            Vec::new(),
            vec![
                Action::AddFlag {
                    flag: "\\seen".to_string(),
                },
                Action::AddFlag {
                    flag: "\\SEEN".to_string(),
                },
            ],
        )]);
        assert_eq!(outcome.flags, vec![Keyword::Seen]);
    }
}
