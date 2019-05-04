use crate::random_access_file::*;
use std::cell::RefCell;
use std::fs::File;
use std::io::prelude::*;

pub struct DiskFile {
    file: RefCell<File>,
    len: usize,
}

impl DiskFile {
    pub fn new(file: File) -> std::io::Result<DiskFile> {
        let len = file.metadata()?.len() as usize;
        Ok(DiskFile {
            file: RefCell::new(file),
            len,
        })
    }
}

impl RandomAccessFile for DiskFile {
    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error> {
        assert!(pos + buf.len() <= self.len());
        let mut file = self.file.borrow_mut();
        file.seek(std::io::SeekFrom::Start(pos as u64))?;
        file.read_exact(buf)?;
        Ok(())
    }
    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error> {
        assert!(pos + buf.len() <= self.len());
        let mut file = self.file.borrow_mut();
        file.seek(std::io::SeekFrom::Start(pos as u64))?;
        file.write_all(buf)?;
        Ok(())
    }
    fn len(&self) -> usize {
        self.len
    }
    fn commit(&self) -> Result<(), Error> {
        self.file.borrow_mut().flush()?;
        Ok(())
    }
}
