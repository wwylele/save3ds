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
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        {
            use std::error::Error;
            println!("Host IO error: {:?}", e);
        }
        Error::IO(e)
    }
}

pub(crate) fn make_error<T>(e: Error) -> Result<T, Error> {
    //println!("Error thrown: {:?}", e);
    // panic!();
    Err(e)
}
