use crate::error::*;
use crate::random_access_file::*;
use std::rc::Rc;

pub trait SdNandFileSystem {
    fn open(&self, path: &[&str]) -> Result<Rc<RandomAccessFile>, Error>;
}
