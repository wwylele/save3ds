use crate::error::*;
use byte_struct::*;
use std::borrow::Borrow;

pub trait RandomAccessFile {
    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error>;
    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error>;
    fn len(&self) -> usize;
    fn commit(&self) -> Result<(), Error>;
}

pub fn read_struct<T: ByteStruct>(f: &RandomAccessFile, pos: usize) -> Result<T, Error> {
    let mut buf = vec![0; T::BYTE_LEN]; // array somehow broken with the associated item as size
    f.borrow().read(pos, &mut buf)?;
    Ok(T::read_bytes(&buf))
}

pub fn write_struct<T: ByteStruct>(f: &RandomAccessFile, pos: usize, data: T) -> Result<(), Error> {
    let mut buf = vec![0; T::BYTE_LEN]; // array somehow broken with the associated item as size
    data.write_bytes(&mut buf);
    f.borrow().write(pos, &buf)?;
    Ok(())
}
