#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("network timeout")]
    Timeout,
    #[error("invalid data response: {0}")]
    InvalidData(String),
    #[error("invalid address for DMA: {0:#x}")]
    InvalidAddress(u64),
    #[error("invalid PCI BDF string: {0}")]
    InvalidBDF(String),
}
