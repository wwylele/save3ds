use crate::error::*;

pub trait FileSystemFile {
    type NameType;
    type DirType;

    fn rename(&mut self, parent: &Self::DirType, name: Self::NameType) -> Result<(), Error>;
    fn get_parent_ino(&self) -> Result<u32, Error>;
    fn get_ino(&self) -> u32;
    fn delete(self) -> Result<(), Error>;
    fn resize(&mut self, len: usize) -> Result<(), Error>;
    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error>;
    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn commit(&self) -> Result<(), Error>;
}

pub trait FileSystemDir {
    type NameType;
    type FileType;

    fn rename(&mut self, parent: &Self, name: Self::NameType) -> Result<(), Error>;
    fn get_parent_ino(&self) -> Result<u32, Error>;
    fn get_ino(&self) -> u32;
    fn open_sub_dir(&self, name: Self::NameType) -> Result<Self, Error>
    where
        Self: Sized;
    fn open_sub_file(&self, name: Self::NameType) -> Result<Self::FileType, Error>;
    fn list_sub_dir(&self) -> Result<Vec<(Self::NameType, u32)>, Error>;
    fn list_sub_file(&self) -> Result<Vec<(Self::NameType, u32)>, Error>;
    fn new_sub_dir(&self, name: Self::NameType) -> Result<Self, Error>
    where
        Self: Sized;
    fn new_sub_file(&self, name: Self::NameType, len: usize) -> Result<Self::FileType, Error>;
    fn delete(self) -> Result<(), Error>;
}

pub trait FileSystem {
    type FileType: FileSystemFile<NameType = Self::NameType, DirType = Self::DirType>;
    type DirType: FileSystemDir<NameType = Self::NameType, FileType = Self::FileType>;
    type NameType;

    fn open_file(&self, ino: u32) -> Result<Self::FileType, Error>;
    fn open_dir(&self, ino: u32) -> Result<Self::DirType, Error>;
    fn open_root(&self) -> Result<Self::DirType, Error> {
        self.open_dir(0)
    }
    fn commit(&self) -> Result<(), Error>;
}
