use crate::error::*;
use crate::random_access_file::*;
use std::rc::Rc;

pub trait SdNandFileSystem {
    fn open(&self, path: &[&str], write: bool) -> Result<Rc<dyn RandomAccessFile>, Error>;
    fn create(&self, path: &[&str], len: usize) -> Result<(), Error>;
    fn remove(&self, path: &[&str]) -> Result<(), Error>;
    fn remove_dir(&self, path: &[&str]) -> Result<(), Error>;
}

#[cfg(test)]
pub mod test {
    use super::*;
    use crate::memory_file::*;
    use std::cell::*;
    use std::collections::HashMap;
    use std::rc::Rc;

    pub struct VirtualFileSystem {
        files: RefCell<HashMap<Vec<String>, Rc<dyn RandomAccessFile>>>,
    }

    impl VirtualFileSystem {
        pub fn new() -> VirtualFileSystem {
            VirtualFileSystem {
                files: RefCell::new(HashMap::new()),
            }
        }
    }

    impl SdNandFileSystem for VirtualFileSystem {
        fn open(&self, path: &[&str], _write: bool) -> Result<Rc<dyn RandomAccessFile>, Error> {
            let path: Vec<_> = path.iter().map(|s| s.to_string()).collect();
            self.files
                .borrow()
                .get(&path)
                .cloned()
                .ok_or(Error::NotFound)
        }
        fn create(&self, path: &[&str], len: usize) -> Result<(), Error> {
            let path: Vec<_> = path.iter().map(|s| s.to_string()).collect();
            self.files
                .borrow_mut()
                .insert(path, Rc::new(MemoryFile::new(vec![0; len])));
            Ok(())
        }
        fn remove(&self, path: &[&str]) -> Result<(), Error> {
            let path: Vec<_> = path.iter().map(|s| s.to_string()).collect();
            let file = self.files.borrow_mut().remove(&path);
            assert!(Rc::strong_count(&file.unwrap()) == 1);
            Ok(())
        }
        fn remove_dir(&self, _path: &[&str]) -> Result<(), Error> {
            Ok(())
        }
    }
}
