pub fn close_completion(tag: &str) -> String {
    format!("{tag} OK CLOSE completed\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_completion_echoes_the_tag() {
        assert_eq!(close_completion("z"), "z OK CLOSE completed\r\n");
    }
}
