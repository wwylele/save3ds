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
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: better UI
        (self as &fmt::Debug).fmt(f)
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
