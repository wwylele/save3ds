use crate::error::*;
use crate::random_access_file::*;
use std::rc::Rc;

pub trait SdNandFileSystem {
    fn open(&self, path: &[&str], write: bool) -> Result<Rc<RandomAccessFile>, Error>;
    fn create(&self, path: &[&str], len: usize) -> Result<(), Error>;
    fn remove(&self, path: &[&str]) -> Result<(), Error>;
}
