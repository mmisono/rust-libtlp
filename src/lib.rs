#![doc = include_str!("../README.md")]
#![warn(rust_2018_idioms)]

pub use crate::error::{Error, ErrorKind};
pub use crate::nettlp::{DmaDirection, NetTlp};
pub mod pci;

mod error;
mod nettlp;
mod tlp;
mod util;
