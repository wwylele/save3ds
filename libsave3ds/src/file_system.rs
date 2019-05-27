use crate::error::*;
use std::rc::Rc;

#[allow(unused_variables)]
pub trait FileSystem {
    type CenterType;
    type FileType;
    type DirType;
    type NameType;

    fn file_open_ino(center: Rc<Self::CenterType>, ino: u32) -> Result<Self::FileType, Error> {
        make_error(Error::Unsupported)
    }

    fn file_rename(
        file: &mut Self::FileType,
        parent: &Self::DirType,
        name: Self::NameType,
    ) -> Result<(), Error> {
        make_error(Error::Unsupported)
    }

    fn file_get_parent_ino(file: &Self::FileType) -> u32;

    fn file_get_ino(file: &Self::FileType) -> u32;

    fn file_delete(file: Self::FileType) -> Result<(), Error> {
        make_error(Error::Unsupported)
    }

    fn resize(file: &mut Self::FileType, len: usize) -> Result<(), Error> {
        make_error(Error::Unsupported)
    }

    fn read(file: &Self::FileType, pos: usize, buf: &mut [u8]) -> Result<(), Error> {
        make_error(Error::Unsupported)
    }

    fn write(file: &Self::FileType, pos: usize, buf: &[u8]) -> Result<(), Error> {
        make_error(Error::Unsupported)
    }

    fn len(file: &Self::FileType) -> usize;

    fn is_empty(file: &Self::FileType) -> bool {
        Self::len(file) == 0
    }

    fn open_root(center: Rc<Self::CenterType>) -> Result<Self::DirType, Error> {
        make_error(Error::Unsupported)
    }

    fn dir_open_ino(center: Rc<Self::CenterType>, ino: u32) -> Result<Self::DirType, Error> {
        make_error(Error::Unsupported)
    }

    fn dir_rename(
        dir: &mut Self::DirType,
        parent: &Self::DirType,
        name: Self::NameType,
    ) -> Result<(), Error> {
        make_error(Error::Unsupported)
    }

    fn dir_get_parent_ino(dir: &Self::DirType) -> u32;

    fn dir_get_ino(dir: &Self::DirType) -> u32;

    fn open_sub_dir(dir: &Self::DirType, name: Self::NameType) -> Result<Self::DirType, Error> {
        make_error(Error::Unsupported)
    }

    fn open_sub_file(dir: &Self::DirType, name: Self::NameType) -> Result<Self::FileType, Error> {
        make_error(Error::Unsupported)
    }

    fn list_sub_dir(dir: &Self::DirType) -> Result<Vec<(Self::NameType, u32)>, Error> {
        make_error(Error::Unsupported)
    }

    fn list_sub_file(dir: &Self::DirType) -> Result<Vec<(Self::NameType, u32)>, Error> {
        make_error(Error::Unsupported)
    }

    fn new_sub_dir(dir: &Self::DirType, name: Self::NameType) -> Result<Self::DirType, Error> {
        make_error(Error::Unsupported)
    }

    fn new_sub_file(
        dir: &Self::DirType,
        name: Self::NameType,
        len: usize,
    ) -> Result<Self::FileType, Error> {
        make_error(Error::Unsupported)
    }

    fn dir_delete(dir: Self::DirType) -> Result<(), Error> {
        make_error(Error::Unsupported)
    }

    fn commit(center: &Self::CenterType) -> Result<(), Error> {
        Ok(())
    }

    fn commit_file(file: &Self::FileType) -> Result<(), Error> {
        Ok(())
    }
}
