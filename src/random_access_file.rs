#[derive(Debug)]
pub enum Error {
    IO(std::io::Error),
    HashMismatch,
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::IO(e)
    }
}

pub trait RandomAccessFile {
    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error>;
    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error>;
    fn len(&self) -> usize;
    fn commit(&self) -> Result<(), Error>;
}
