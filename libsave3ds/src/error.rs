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
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::IO(e)
    }
}

pub(crate) fn make_error<T>(e: Error) -> Result<T, Error> {
    //println!("Error thrown: {:?}", e);
    // panic!();
    Err(e)
}
