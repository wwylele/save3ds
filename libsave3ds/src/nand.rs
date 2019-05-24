use crate::disk_file::DiskFile;
use crate::error::*;
use crate::random_access_file::*;
use crate::sd_nand_common::*;
use std::path::*;
use std::rc::Rc;

pub struct Nand {
    path: PathBuf,
}

impl Nand {
    pub fn new(nand_path: &str) -> Result<Nand, Error> {
        let path = PathBuf::from(nand_path);
        Ok(Nand { path })
    }
}

impl SdNandFileSystem for Nand {
    fn open(&self, path: &[&str], write: bool) -> Result<Rc<RandomAccessFile>, Error> {
        let file_path = path.iter().fold(self.path.clone(), |a, b| a.join(b));

        let file = DiskFile::new(
            std::fs::OpenOptions::new()
                .read(true)
                .write(write)
                .open(file_path)?,
        )?;

        Ok(Rc::new(file))
    }
}
