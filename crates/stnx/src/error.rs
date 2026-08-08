use miette::Diagnostic;
use thiserror::Error;

#[derive(Error, Diagnostic, Debug)]
#[error("Lexing error: {message}")]
#[diagnostic(code(stnx::lexer_error))]
pub struct LexError {
    #[source_code]
    pub src: String,
    #[label("invalid token here")]
    pub span: miette::SourceSpan,
    pub message: String,
}

impl LexError {
    pub fn new(src: &str, offset: usize, len: usize, message: impl Into<String>) -> Self {
        Self {
            src: src.to_string(),
            span: (offset, len).into(),
            message: message.into(),
        }
    }
}

#[derive(Error, Diagnostic, Debug)]
#[error("Parse error: {message}")]
#[diagnostic(code(stnx::parse_error))]
pub struct ParseError {
    #[source_code]
    pub src: String,
    #[label("{message}")]
    pub span: miette::SourceSpan,
    pub message: String,
}

#[derive(Error, Debug)]
#[error("{message}")]
pub struct SemanticError {
    pub message: String,
}

#[derive(Error, Debug)]
#[error("{message}")]
pub struct TypeError {
    pub message: String,
}

#[derive(Error, Debug)]
#[error("{message}")]
pub struct CodegenError {
    pub message: String,
}

#[derive(Error, Debug)]
pub struct TargetError {
    pub message: String,
    pub triple: Option<String>,
}

impl std::fmt::Display for TargetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "target error: {}", self.message)
    }
}

impl TargetError {
    pub fn target_init_failed(message: impl Into<String>, triple: Option<&str>) -> Self {
        Self {
            message: message.into(),
            triple: triple.map(|s| s.to_string()),
        }
    }

    pub fn target_lookup_failed(triple: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            triple: Some(triple.into()),
            message: message.into(),
        }
    }

    pub fn target_machine_failed(triple: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            triple: Some(triple.into()),
            message: message.into(),
        }
    }
}

#[derive(Error, Debug)]
pub struct LinkError {
    pub message: String,
    pub details: Option<String>,
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "link error: {}", self.message)
    }
}

impl LinkError {
    pub fn linker_not_found(linker: impl Into<String>) -> Self {
        let linker_name = linker.into();
        Self {
            message: format!("linker '{}' not found", linker_name),
            details: None,
        }
    }

    pub fn linking_failed(output: impl Into<String>, details: Option<String>) -> Self {
        let output_name = output.into();
        Self {
            message: format!("linking failed for output: {}", output_name),
            details,
        }
    }
}

#[derive(Error, Debug)]
pub enum CompilerError {
    #[error("lexer error")]
    Lexer(#[from] LexError),
    #[error("parse error")]
    Parse(String),
    #[error("semantic error: {0}")]
    Semantic(String),
    #[error("type error: {0}")]
    Type(String),
    #[error("code generation error: {0}")]
    Codegen(String),
    #[error("{0}")]
    Target(#[from] TargetError),
    #[error("{0}")]
    Link(#[from] LinkError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("process error: {0}")]
    Process(String),

    // Structured codegen errors with richer context
    #[error("IR emission error: {message}")]
    IrEmissionError { message: String },
}

// Backward-compatible field aliases so existing code that uses
// `CompilerError::Semantic { message }` or `CompilerError::Codegen { message }`
// continues to work. We provide constructors and conversion helpers.

impl CompilerError {
    pub fn semantic(message: impl Into<String>) -> Self {
        CompilerError::Semantic(message.into())
    }

    pub fn codegen(message: impl Into<String>) -> Self {
        CompilerError::Codegen(message.into())
    }
}

pub type CompilerResult<T> = Result<T, CompilerError>;

impl From<miette::Report> for CompilerError {
    fn from(e: miette::Report) -> Self {
        CompilerError::semantic(e.to_string())
    }
}

pub type TargetResult<T> = Result<T, TargetError>;
pub type LinkResult<T> = Result<T, LinkError>;
