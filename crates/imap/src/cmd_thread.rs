#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadAlgorithm {
    References,
    OrderedSubject,
}

pub fn parse_algorithm(word: &str) -> Option<ThreadAlgorithm> {
    match word.to_ascii_uppercase().as_str() {
        "REFERENCES" => Some(ThreadAlgorithm::References),
        "ORDEREDSUBJECT" => Some(ThreadAlgorithm::OrderedSubject),
        _ => None,
    }
}

pub fn thread_response(groups: &[Vec<u32>]) -> String {
    let mut line = String::from("* THREAD");
    if !groups.is_empty() {
        line.push(' ');
    }
    for group in groups {
        line.push('(');
        line.push_str(
            &group
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(" "),
        );
        line.push(')');
    }
    line.push_str("\r\n");
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn algorithms_parse_case_insensitively() {
        assert_eq!(
            parse_algorithm("references"),
            Some(ThreadAlgorithm::References)
        );
        assert_eq!(
            parse_algorithm("ORDEREDSUBJECT"),
            Some(ThreadAlgorithm::OrderedSubject)
        );
        assert_eq!(parse_algorithm("SUBJECT"), None);
    }

    #[test]
    fn groups_render_as_parenthesised_lists() {
        assert_eq!(
            thread_response(&[vec![1, 3], vec![2]]),
            "* THREAD (1 3)(2)\r\n"
        );
        assert_eq!(thread_response(&[]), "* THREAD\r\n");
    }
}
