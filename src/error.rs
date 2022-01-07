#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("network timeout")]
    Timeout,
    #[error("invalid PCI BDF string: {0}")]
    InvalidBDF(String),
}
