use tree_sitter::Language as TsLanguage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Go,
    C,
    Cpp,
    Java,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryKind {
    Defs,
    Imports,
    Refs,
}

impl Language {
    pub fn name(self) -> &'static str {
        match self {
            Language::Rust => "rust",
            Language::TypeScript => "typescript",
            Language::Tsx => "tsx",
            Language::JavaScript => "javascript",
            Language::Python => "python",
            Language::Go => "go",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::Java => "java",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "rs" => Some(Language::Rust),
            "ts" => Some(Language::TypeScript),
            "tsx" => Some(Language::Tsx),
            "js" | "mjs" | "cjs" => Some(Language::JavaScript),
            "py" => Some(Language::Python),
            "go" => Some(Language::Go),
            "c" | "h" => Some(Language::C),
            "cc" | "cpp" | "cxx" | "hpp" | "hh" => Some(Language::Cpp),
            "java" => Some(Language::Java),
            _ => None,
        }
    }

    pub fn ts_language(self) -> TsLanguage {
        match self {
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Language::Python => tree_sitter_python::LANGUAGE.into(),
            Language::Go => tree_sitter_go::LANGUAGE.into(),
            Language::C => tree_sitter_c::LANGUAGE.into(),
            Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Language::Java => tree_sitter_java::LANGUAGE.into(),
        }
    }

    /// Returns the tree-sitter S-expression query source for the given language and kind.
    /// All combinations are now wired; the function always returns `Some`.
    pub fn query_source(self, kind: QueryKind) -> Option<&'static str> {
        match (self, kind) {
            (Language::Rust, QueryKind::Defs) => Some(include_str!("queries/rust_defs.scm")),
            (Language::Rust, QueryKind::Imports) => Some(include_str!("queries/rust_imports.scm")),
            (Language::Rust, QueryKind::Refs) => Some(include_str!("queries/rust_refs.scm")),
            (Language::TypeScript | Language::Tsx, QueryKind::Defs) => {
                Some(include_str!("queries/typescript_defs.scm"))
            }
            (Language::TypeScript | Language::Tsx, QueryKind::Imports) => {
                Some(include_str!("queries/typescript_imports.scm"))
            }
            (Language::TypeScript | Language::Tsx, QueryKind::Refs) => {
                Some(include_str!("queries/typescript_refs.scm"))
            }
            (Language::JavaScript, QueryKind::Defs) => {
                Some(include_str!("queries/javascript_defs.scm"))
            }
            (Language::JavaScript, QueryKind::Imports) => {
                Some(include_str!("queries/javascript_imports.scm"))
            }
            (Language::JavaScript, QueryKind::Refs) => {
                Some(include_str!("queries/javascript_refs.scm"))
            }
            (Language::Python, QueryKind::Defs) => Some(include_str!("queries/python_defs.scm")),
            (Language::Python, QueryKind::Imports) => {
                Some(include_str!("queries/python_imports.scm"))
            }
            (Language::Python, QueryKind::Refs) => Some(include_str!("queries/python_refs.scm")),
            (Language::Go, QueryKind::Defs) => Some(include_str!("queries/go_defs.scm")),
            (Language::Go, QueryKind::Imports) => Some(include_str!("queries/go_imports.scm")),
            (Language::Go, QueryKind::Refs) => Some(include_str!("queries/go_refs.scm")),
            (Language::C, QueryKind::Defs) => Some(include_str!("queries/c_defs.scm")),
            (Language::C, QueryKind::Imports) => Some(include_str!("queries/c_imports.scm")),
            (Language::C, QueryKind::Refs) => Some(include_str!("queries/c_refs.scm")),
            (Language::Cpp, QueryKind::Defs) => Some(include_str!("queries/cpp_defs.scm")),
            (Language::Cpp, QueryKind::Imports) => Some(include_str!("queries/cpp_imports.scm")),
            (Language::Cpp, QueryKind::Refs) => Some(include_str!("queries/cpp_refs.scm")),
            (Language::Java, QueryKind::Defs) => Some(include_str!("queries/java_defs.scm")),
            (Language::Java, QueryKind::Imports) => Some(include_str!("queries/java_imports.scm")),
            (Language::Java, QueryKind::Refs) => Some(include_str!("queries/java_refs.scm")),
        }
    }
}
