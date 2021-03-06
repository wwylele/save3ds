use log::*;
use std::fmt;

#[derive(Debug)]
pub enum Error {
    IO(std::io::Error),
    HashMismatch,
    OutOfBound,
    MagicMismatch,
    SizeMismatch,
    InvalidValue,
    BrokenFat,
    NoSpace,
    NotFound,
    AlreadyExist,
    DeletingRoot,
    SignatureMismatch,
    MissingBoot9,
    MissingSd,
    MissingNand,
    MissingGame,
    MissingPriv,
    MissingKeyY2F,
    MissingKeyX19,
    MissingKeyX1A,
    MissingOtp,
    BrokenSd,
    NotEmpty,
    Unsupported,
    UniqueIdMismatch,
    BrokenOtp,
    Busy,
    BrokenGame,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::IO(e) => write!(f, "IO error from host file system: {:?}", e),
            Error::HashMismatch => write!(
                f,
                "SHA256 mismatch, caused by either corrupted data or uninitialized data"
            ),
            Error::OutOfBound => write!(f, "Out-of-bound access, caused by corrupted data"),
            Error::MagicMismatch => write!(f, "Magic mismatch, caused by corrupted data"),
            Error::SizeMismatch => write!(f, "Size mismatch, caused by corrupted data"),
            Error::InvalidValue => write!(f, "Invalid value, caused by corrupted data"),
            Error::BrokenFat => write!(f, "Broken FAT,  caused by corrupted data"),
            Error::NoSpace => write!(f, "Insufficient space for the operation"),
            Error::NotFound => write!(f, "The requested file or directory is not found"),
            Error::AlreadyExist => write!(f, "The file or directory to create already exists"),
            Error::DeletingRoot => write!(f, "Trying to delete the root directory"),
            Error::SignatureMismatch => write!(f, "Signature mismatch, caused by corrupted data"),
            Error::MissingBoot9 => write!(f, "Missing boot9.bin"),
            Error::MissingSd => write!(f, "Cannot open SD due to missing SD or movable.sed"),
            Error::MissingNand => write!(f, "Missing NAND"),
            Error::MissingGame => write!(f, "Missing game"),
            Error::MissingPriv => write!(f, "Missing private header"),
            Error::MissingKeyY2F => write!(f, "Missing 0x2F key Y"),
            Error::MissingKeyX19 => write!(f, "Missing 0x19 key X"),
            Error::MissingKeyX1A => write!(f, "Missing 0x1A key X"),
            Error::MissingOtp => write!(f, "Missing OTP"),
            Error::BrokenSd => write!(f, "Corrupted SD"),
            Error::NotEmpty => write!(f, "Trying to delete a non-empty directory"),
            Error::Unsupported => write!(f, "The operation is not supported on this archive"),
            Error::UniqueIdMismatch => {
                write!(f, "Extdata unique ID mismatch, caused by corrupted data")
            }
            Error::BrokenOtp => write!(f, "Corrupted OTP"),
            Error::Busy => write!(
                f,
                "The file or directory is currently used by other program"
            ),
            Error::BrokenGame => write!(f, "Provided game file is broken"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        error!("Host IO error: {:?}", e);
        Error::IO(e)
    }
}

pub(crate) fn make_error<T>(e: Error) -> Result<T, Error> {
    info!("Error thrown: {:?}", e);
    Err(e)
}
