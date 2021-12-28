#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
}

impl Error {
    pub(crate) fn new(kind: ErrorKind) -> Error {
        Error { kind }
    }

    pub fn kind(&self) -> &ErrorKind {
        &self.kind
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ErrorKind {
    Io(std::io::Error),
    InvalidBDF(String),
}

impl std::error::Error for Error {
    fn description(&self) -> &str {
        match self.kind {
            ErrorKind::Io(_) => "IO error",
            ErrorKind::InvalidBDF(_) => "invalid PCI BDF string",
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            ErrorKind::Io(ref e) => e.fmt(f),
            ErrorKind::InvalidBDF(ref s) => write!(f, "{}", s),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::new(ErrorKind::Io(e))
    }
}
