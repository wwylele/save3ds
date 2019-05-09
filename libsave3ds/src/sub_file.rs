use crate::error::*;
use crate::random_access_file::*;
use std::rc::Rc;

pub struct SubFile {
    parent: Rc<RandomAccessFile>,
    begin: usize,
    len: usize,
}

impl SubFile {
    pub fn new(parent: Rc<RandomAccessFile>, begin: usize, len: usize) -> Result<SubFile, Error> {
        if begin + len > parent.len() {
            return make_error(Error::OutOfBound);
        }
        Ok(SubFile { parent, begin, len })
    }
}

impl RandomAccessFile for SubFile {
    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error> {
        if pos + buf.len() > self.len() {
            return make_error(Error::OutOfBound);
        }
        self.parent.read(pos + self.begin, buf)
    }
    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error> {
        if pos + buf.len() > self.len() {
            return make_error(Error::OutOfBound);
        }
        self.parent.write(pos + self.begin, buf)
    }
    fn len(&self) -> usize {
        self.len
    }
    fn commit(&self) -> Result<(), Error> {
        Ok(())
    }
}
