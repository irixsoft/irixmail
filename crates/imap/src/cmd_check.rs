pub fn check_completion(tag: &str) -> String {
    format!("{tag} OK CHECK completed\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_completion_echoes_the_tag() {
        assert_eq!(check_completion("q"), "q OK CHECK completed\r\n");
    }
}
