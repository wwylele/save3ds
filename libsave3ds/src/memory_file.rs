use crate::error::*;
use crate::random_access_file::*;
use std::cell::RefCell;

/// Implements `RandomAccessFile` as a simple Vec<u8>
pub struct MemoryFile {
    data: RefCell<Vec<u8>>,
}

impl MemoryFile {
    pub fn new(data: Vec<u8>) -> MemoryFile {
        MemoryFile {
            data: RefCell::new(data),
        }
    }

    /// Creates a `MemoryFile` that clones the content from another `RandomAccessFile`
    pub fn from_file(file: &dyn RandomAccessFile) -> Result<MemoryFile, Error> {
        let mut data = vec![0; file.len()];
        file.read(0, &mut data)?;
        Ok(MemoryFile {
            data: RefCell::new(data),
        })
    }
}

impl RandomAccessFile for MemoryFile {
    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error> {
        let data = self.data.borrow();
        if pos + buf.len() > data.len() {
            return make_error(Error::OutOfBound);
        }
        buf.copy_from_slice(&data[pos..pos + buf.len()]);
        Ok(())
    }
    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error> {
        let mut data = self.data.borrow_mut();
        if pos + buf.len() > data.len() {
            return make_error(Error::OutOfBound);
        }
        data[pos..pos + buf.len()].copy_from_slice(buf);
        Ok(())
    }
    fn len(&self) -> usize {
        self.data.borrow().len()
    }
    fn commit(&self) -> Result<(), Error> {
        Ok(())
    }
}

#[test]
fn test() {
    let file = MemoryFile::new(vec![9, 9, 9, 9, 9, 9, 9, 9, 9]);
    let buf = [1, 3, 5, 7];
    file.write(2, &buf).unwrap();
    file.write(4, &buf).unwrap();
    let mut buf2 = [0; 7];
    file.read(2, &mut buf2).unwrap();
    assert_eq!(buf2, [1, 3, 1, 3, 5, 7, 9]);
}
