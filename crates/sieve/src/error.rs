#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for CompileError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_compile_error_displays_its_line_and_message() {
        let error = CompileError {
            line: 3,
            column: 7,
            message: "unknown command".into(),
        };
        assert_eq!(error.to_string(), "line 3: unknown command");
    }
}
