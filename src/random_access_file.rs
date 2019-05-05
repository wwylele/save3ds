use byte_struct::*;

#[derive(Debug)]
pub enum Error {
    IO(std::io::Error),
    HashMismatch,
    OutOfBound,
    MagicMismatch,
    SizeMismatch,
    InvalidValue,
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::IO(e)
    }
}

pub fn make_error<T>(e: Error) -> Result<T, Error> {
    //println!("Error thrown: {:?}", e);
    Err(e)
}

pub trait RandomAccessFile {
    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error>;
    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error>;
    fn len(&self) -> usize;
    fn commit(&self) -> Result<(), Error>;
}

impl RandomAccessFile {
    pub fn read_struct<T: ByteStruct>(&self, pos: usize) -> Result<T, Error> {
        let mut buf = vec![0; T::BYTE_LEN]; // array somehow broken with the associated item as size
        self.read(pos, &mut buf)?;
        Ok(T::read_bytes(&buf))
    }

    pub fn write_struct<T: ByteStruct>(&self, pos: usize, data: T) -> Result<(), Error> {
        let mut buf = vec![0; T::BYTE_LEN]; // array somehow broken with the associated item as size
        data.write_bytes(&mut buf);
        self.write(pos, &buf)?;
        Ok(())
    }
}
