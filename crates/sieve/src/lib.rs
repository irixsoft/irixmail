mod error;
mod grammar;
mod instruction;
mod lexer;
mod limits;
mod matching;
mod message;
mod runtime;
mod string;

pub use error::CompileError;
pub use limits::Limits;

pub const CAPABILITIES: &[&str] = &[
    "fileinto",
    "envelope",
    "imap4flags",
    "encoded-character",
    "comparator-i;octet",
];

#[derive(Default)]
pub struct Compiler {
    limits: limits::CompilerLimits,
}

impl Compiler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn compile(&self, source: &str) -> Result<Script, CompileError> {
        if source.len() > self.limits.max_script_size {
            return Err(CompileError {
                line: 1,
                column: 1,
                message: "script exceeds the maximum size".into(),
            });
        }
        let tokens = lexer::tokenize(source, &self.limits)?;
        let program = grammar::compile(tokens, &self.limits)?;
        Ok(Script {
            instructions: program.instructions,
            capabilities: program.capabilities,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    pub(crate) instructions: Vec<instruction::Instruction>,
    capabilities: Vec<String>,
}

impl Script {
    pub fn capabilities(&self) -> &[String] {
        &self.capabilities
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Envelope<'a> {
    pub from: &'a str,
    pub to: &'a str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub keep: bool,
    pub discarded: bool,
    pub file_into: Vec<String>,
    pub redirects: Vec<String>,
    pub flags: Vec<String>,
}

pub fn execute(script: &Script, raw: &[u8], envelope: Envelope<'_>, limits: &Limits) -> Outcome {
    runtime::run(script, raw, envelope, limits)
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod golden {
    use super::*;

    #[test]
    fn every_testdata_script_compiles_to_its_golden_instructions() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata");
        let compiler = Compiler::new();
        let mut checked = 0usize;
        for entry in std::fs::read_dir(&root).expect("testdata directory exists") {
            let path = entry.expect("readable entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("sieve") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("readable script");
            let script = compiler
                .compile(&source)
                .unwrap_or_else(|e| panic!("{} failed to compile: {e}", path.display()));
            let listing: Vec<(usize, &instruction::Instruction)> =
                script.instructions.iter().enumerate().collect();
            let actual = serde_json::to_string_pretty(&listing).expect("serializable");
            let golden_path = path.with_extension("json");
            if std::env::var_os("SIEVE_BLESS").is_some() {
                std::fs::write(&golden_path, &actual).expect("golden written");
                checked += 1;
                continue;
            }
            let expected = std::fs::read_to_string(&golden_path)
                .unwrap_or_else(|_| panic!("{} is missing", golden_path.display()));
            if actual != expected {
                let failed = path.with_extension("failed");
                std::fs::write(&failed, &actual).expect("failure snapshot written");
                panic!("{} does not match its golden file", path.display());
            }
            checked += 1;
        }
        assert!(
            checked >= 8,
            "expected at least 8 golden scripts, ran {checked}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_oversized_script_is_rejected_before_lexing() {
        let compiler = Compiler::new();
        let source = format!("# {}", "x".repeat(200 * 1024));
        assert_eq!(
            compiler.compile(&source).unwrap_err().message,
            "script exceeds the maximum size"
        );
    }

    #[test]
    fn a_compiled_script_reports_its_required_capabilities() {
        let script = Compiler::new()
            .compile("require [\"fileinto\", \"envelope\"];\nkeep;")
            .unwrap();
        assert_eq!(script.capabilities(), ["fileinto", "envelope"]);
    }

    #[test]
    fn the_advertised_capability_list_covers_the_supported_extensions() {
        for capability in CAPABILITIES {
            let source = format!("require \"{capability}\";\nkeep;");
            assert!(Compiler::new().compile(&source).is_ok(), "{capability}");
        }
    }
}
