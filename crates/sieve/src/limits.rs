#[derive(Debug, Clone)]
pub(crate) struct CompilerLimits {
    pub max_script_size: usize,
    pub max_string_size: usize,
    pub max_nested_blocks: usize,
    pub max_nested_tests: usize,
    pub max_list_items: usize,
}

impl Default for CompilerLimits {
    fn default() -> Self {
        Self {
            max_script_size: 128 * 1024,
            max_string_size: 4096,
            max_nested_blocks: 15,
            max_nested_tests: 15,
            max_list_items: 128,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Limits {
    pub cpu: u32,
    pub max_redirects: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            cpu: 5000,
            max_redirects: 4,
        }
    }
}
