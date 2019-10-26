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
    Missing,
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
            Error::Missing => write!(
                f,
                "Provided resource (SD, NAND, OTP etc.) is insufficient for opening the archive"
            ),
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
        {
            println!("Host IO error: {:?}", e);
        }
        Error::IO(e)
    }
}

pub(crate) fn make_error<T>(e: Error) -> Result<T, Error> {
    // println!("Error thrown: {:?}", e);
    Err(e)
}
