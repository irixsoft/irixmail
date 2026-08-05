use crate::instruction::{EnvelopePart, Instruction, Test};
use crate::limits::Limits;
use crate::matching::value_matches;
use crate::message::{address_part, parse_envelope_address, MessageView};
use crate::{Envelope, Outcome, Script};

pub(crate) fn run(script: &Script, raw: &[u8], envelope: Envelope<'_>, limits: &Limits) -> Outcome {
    let message = MessageView::new(raw);
    let env_from = parse_envelope_address(envelope.from);
    let env_to = parse_envelope_address(envelope.to);
    let mut pos = 0usize;
    let mut steps = 0u32;
    let mut test_result = false;
    let mut implicit_keep = true;
    let mut explicit_keep = false;
    let mut discard_requested = false;
    let mut file_into: Vec<String> = Vec::new();
    let mut redirects: Vec<String> = Vec::new();
    let mut flags: Vec<String> = Vec::new();
    while pos < script.instructions.len() {
        steps += 1;
        if steps > limits.cpu {
            break;
        }
        match &script.instructions[pos] {
            Instruction::Test(test) => {
                test_result = eval(test, &message, env_from.as_deref(), env_to.as_deref());
            }
            Instruction::Jmp(target) => {
                pos = *target;
                continue;
            }
            Instruction::Jz(target) => {
                if !test_result {
                    pos = *target;
                    continue;
                }
            }
            Instruction::Jnz(target) => {
                if test_result {
                    pos = *target;
                    continue;
                }
            }
            Instruction::Keep => explicit_keep = true,
            Instruction::Discard => {
                discard_requested = true;
                implicit_keep = false;
            }
            Instruction::Stop => break,
            Instruction::FileInto(mailbox) => {
                implicit_keep = false;
                if !file_into.contains(mailbox) {
                    file_into.push(mailbox.clone());
                }
            }
            Instruction::Redirect(address) => {
                if redirects.len() < limits.max_redirects
                    && !redirects.iter().any(|r| r.eq_ignore_ascii_case(address))
                {
                    redirects.push(address.clone());
                }
            }
            Instruction::AddFlag(new_flags) => add_flags(&mut flags, new_flags),
            Instruction::SetFlag(new_flags) => {
                flags.clear();
                add_flags(&mut flags, new_flags);
            }
            Instruction::RemoveFlag(removed) => {
                flags.retain(|flag| !removed.iter().any(|r| r.eq_ignore_ascii_case(flag)));
            }
        }
        pos += 1;
    }
    let keep = explicit_keep || implicit_keep;
    Outcome {
        keep,
        discarded: discard_requested && !keep && file_into.is_empty(),
        file_into,
        redirects,
        flags,
    }
}

fn add_flags(flags: &mut Vec<String>, new_flags: &[String]) {
    for flag in new_flags {
        if !flags.iter().any(|f| f.eq_ignore_ascii_case(flag)) {
            flags.push(flag.clone());
        }
    }
}

fn eval(
    test: &Test,
    message: &MessageView<'_>,
    env_from: Option<&str>,
    env_to: Option<&str>,
) -> bool {
    match test {
        Test::Bool(value) => *value,
        Test::Header {
            headers,
            keys,
            match_type,
            comparator,
            is_not,
        } => {
            let result = headers.iter().any(|header| {
                message.header_values(header).iter().any(|value| {
                    keys.iter()
                        .any(|key| value_matches(*match_type, *comparator, key, value))
                })
            });
            result ^ is_not
        }
        Test::Address {
            headers,
            keys,
            part,
            match_type,
            comparator,
            is_not,
        } => {
            let result = headers.iter().any(|header| {
                message.header_addresses(header, *part).iter().any(|value| {
                    keys.iter()
                        .any(|key| value_matches(*match_type, *comparator, key, value))
                })
            });
            result ^ is_not
        }
        Test::Envelope {
            parts,
            keys,
            part,
            match_type,
            comparator,
            is_not,
        } => {
            let result = parts.iter().any(|envelope_part| {
                let value = match envelope_part {
                    EnvelopePart::From => env_from,
                    EnvelopePart::To => env_to,
                };
                value.is_some_and(|value| {
                    let value = address_part(value, *part);
                    keys.iter()
                        .any(|key| value_matches(*match_type, *comparator, key, &value))
                })
            });
            result ^ is_not
        }
        Test::Exists { headers, is_not } => {
            headers.iter().all(|header| message.header_exists(header)) ^ is_not
        }
        Test::Size {
            over,
            limit,
            is_not,
        } => {
            let result = if *over {
                message.size() > *limit
            } else {
                message.size() < *limit
            };
            result ^ is_not
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{execute, Compiler, Envelope, Limits, Outcome};

    const MESSAGE: &[u8] = concat!(
        "From: Weekly News <newsletter@example.com>\r\n",
        "To: me@example.org\r\n",
        "Subject: Weekly deals\r\n",
        "\r\n",
        "Hello!\r\n"
    )
    .as_bytes();

    fn run(source: &str) -> Outcome {
        let script = Compiler::new().compile(source).unwrap();
        execute(
            &script,
            MESSAGE,
            Envelope {
                from: "newsletter@example.com",
                to: "me@example.org",
            },
            &Limits::default(),
        )
    }

    #[test]
    fn an_empty_script_keeps_the_message() {
        let outcome = run("");
        assert!(outcome.keep);
        assert!(!outcome.discarded);
        assert!(outcome.file_into.is_empty());
        assert!(outcome.redirects.is_empty());
        assert!(outcome.flags.is_empty());
    }

    #[test]
    fn a_matching_fileinto_cancels_the_implicit_keep() {
        let outcome = run(concat!(
            "require \"fileinto\";\n",
            "if header :contains \"From\" \"newsletter\" { fileinto \"Newsletters\"; }"
        ));
        assert!(!outcome.keep);
        assert_eq!(outcome.file_into, vec!["Newsletters"]);
    }

    #[test]
    fn a_non_matching_test_leaves_the_implicit_keep() {
        let outcome = run(concat!(
            "require \"fileinto\";\n",
            "if header :contains \"From\" \"nobody\" { fileinto \"Newsletters\"; }"
        ));
        assert!(outcome.keep);
        assert!(outcome.file_into.is_empty());
    }

    #[test]
    fn discard_drops_the_message() {
        let outcome = run("discard;");
        assert!(!outcome.keep);
        assert!(outcome.discarded);
    }

    #[test]
    fn an_explicit_keep_after_discard_still_delivers() {
        let outcome = run("discard;\nkeep;");
        assert!(outcome.keep);
        assert!(!outcome.discarded);
    }

    #[test]
    fn redirect_does_not_cancel_the_implicit_keep() {
        let outcome = run("redirect \"elsewhere@example.net\";");
        assert!(outcome.keep);
        assert!(!outcome.discarded);
        assert_eq!(outcome.redirects, vec!["elsewhere@example.net"]);
    }

    #[test]
    fn discard_still_collects_redirects() {
        let outcome = run("redirect \"elsewhere@example.net\";\ndiscard;");
        assert!(outcome.discarded);
        assert_eq!(outcome.redirects, vec!["elsewhere@example.net"]);
    }

    #[test]
    fn the_implicit_keep_carries_accumulated_flags() {
        let outcome = run("require \"imap4flags\";\naddflag \"\\\\Flagged\";");
        assert!(outcome.keep);
        assert_eq!(outcome.flags, vec!["\\Flagged"]);
    }

    #[test]
    fn flags_travel_with_a_fileinto() {
        let outcome = run(concat!(
            "require [\"fileinto\", \"imap4flags\"];\n",
            "addflag \"Important\";\nfileinto \"Priority\";"
        ));
        assert_eq!(outcome.file_into, vec!["Priority"]);
        assert_eq!(outcome.flags, vec!["Important"]);
    }

    #[test]
    fn setflag_replaces_and_removeflag_deletes_case_insensitively() {
        let outcome = run(concat!(
            "require \"imap4flags\";\n",
            "addflag \"one two\";\nsetflag \"Three Four\";\nremoveflag \"THREE\";"
        ));
        assert_eq!(outcome.flags, vec!["Four"]);
    }

    #[test]
    fn duplicate_flags_and_targets_are_collapsed() {
        let outcome = run(concat!(
            "require [\"fileinto\", \"imap4flags\"];\n",
            "addflag \"A\";\naddflag \"a\";\nfileinto \"X\";\nfileinto \"X\";\n",
            "redirect \"a@b.example\";\nredirect \"A@B.EXAMPLE\";"
        ));
        assert_eq!(outcome.flags, vec!["A"]);
        assert_eq!(outcome.file_into, vec!["X"]);
        assert_eq!(outcome.redirects, vec!["a@b.example"]);
    }

    #[test]
    fn stop_halts_before_later_commands() {
        let outcome = run(concat!(
            "require \"fileinto\";\n",
            "fileinto \"First\";\nstop;\nfileinto \"Second\";"
        ));
        assert_eq!(outcome.file_into, vec!["First"]);
    }

    #[test]
    fn elsif_and_else_branches_run_exclusively() {
        let outcome = run(concat!(
            "require \"fileinto\";\n",
            "if header :is \"subject\" \"nope\" { fileinto \"A\"; }\n",
            "elsif header :contains \"subject\" \"deals\" { fileinto \"B\"; }\n",
            "else { fileinto \"C\"; }"
        ));
        assert_eq!(outcome.file_into, vec!["B"]);
    }

    #[test]
    fn envelope_tests_match_the_sanitized_envelope() {
        let outcome = run(concat!(
            "require [\"envelope\", \"fileinto\"];\n",
            "if envelope :domain :is \"from\" \"example.com\" { fileinto \"FromExample\"; }"
        ));
        assert_eq!(outcome.file_into, vec!["FromExample"]);
    }

    #[test]
    fn a_junk_envelope_value_never_matches() {
        let script = Compiler::new()
            .compile(concat!(
                "require \"envelope\";\n",
                "if envelope :is \"from\" [\"\", \"*\"] { discard; }"
            ))
            .unwrap();
        let outcome = crate::execute(
            &script,
            MESSAGE,
            Envelope {
                from: "not an address",
                to: "me@example.org",
            },
            &Limits::default(),
        );
        assert!(outcome.keep);
        assert!(!outcome.discarded);
    }

    #[test]
    fn a_null_envelope_sender_matches_the_empty_string() {
        let script = Compiler::new()
            .compile(concat!(
                "require \"envelope\";\n",
                "if envelope :is \"from\" \"\" { discard; }"
            ))
            .unwrap();
        let outcome = crate::execute(
            &script,
            MESSAGE,
            Envelope {
                from: "<>",
                to: "me@example.org",
            },
            &Limits::default(),
        );
        assert!(outcome.discarded);
    }

    #[test]
    fn size_tests_compare_the_raw_message_length() {
        let over = run(&format!(
            "if size :over {} {{ discard; }}",
            MESSAGE.len() - 1
        ));
        assert!(over.discarded);
        let under = run(&format!(
            "if size :under {} {{ discard; }}",
            MESSAGE.len() + 1
        ));
        assert!(under.discarded);
        let not_over = run(&format!("if size :over {} {{ discard; }}", MESSAGE.len()));
        assert!(not_over.keep);
    }

    #[test]
    fn exists_requires_every_listed_header() {
        assert!(run("if exists [\"from\", \"subject\"] { discard; }").discarded);
        assert!(run("if exists [\"from\", \"x-nope\"] { discard; }").keep);
        assert!(run("if not exists \"x-nope\" { discard; }").discarded);
    }

    #[test]
    fn matches_globs_against_header_values() {
        assert!(run("if header :matches \"subject\" \"*deals*\" { discard; }").discarded);
        assert!(run("if header :matches \"subject\" \"deals\" { discard; }").keep);
    }

    #[test]
    fn redirects_beyond_the_limit_are_ignored() {
        let source = (0..6)
            .map(|i| format!("redirect \"r{i}@example.net\";"))
            .collect::<Vec<_>>()
            .join("\n");
        let outcome = run(&source);
        assert_eq!(outcome.redirects.len(), Limits::default().max_redirects);
    }

    #[test]
    fn the_cpu_limit_stops_execution_but_keeps_the_message() {
        let source = "keep;\n".repeat(100);
        let script = Compiler::new().compile(&source).unwrap();
        let outcome = crate::execute(
            &script,
            MESSAGE,
            Envelope {
                from: "a@b.example",
                to: "me@example.org",
            },
            &Limits {
                cpu: 10,
                max_redirects: 4,
            },
        );
        assert!(outcome.keep);
        assert!(!outcome.discarded);
    }

    #[test]
    fn an_unparseable_message_still_follows_the_script() {
        let script = Compiler::new()
            .compile("if exists \"from\" { discard; }")
            .unwrap();
        let outcome = crate::execute(
            &script,
            b"",
            Envelope {
                from: "a@b.example",
                to: "me@example.org",
            },
            &Limits::default(),
        );
        assert!(outcome.keep);
    }
}
