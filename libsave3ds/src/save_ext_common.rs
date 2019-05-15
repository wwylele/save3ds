use crate::error::*;
use crate::fs_meta::*;
use byte_struct::*;
use std::rc::Rc;

#[derive(ByteStruct, Clone)]
#[byte_struct_le]
pub(crate) struct SaveExtDir {
    pub next: u32,
    pub sub_dir: u32,
    pub sub_file: u32,
    pub padding: u32,
}

#[derive(ByteStruct, Clone, PartialEq)]
#[byte_struct_le]
pub(crate) struct SaveExtKey {
    parent: u32,
    name: [u8; 16],
}

impl DirInfo for SaveExtDir {
    fn set_sub_dir(&mut self, index: u32) {
        self.sub_dir = index;
    }
    fn get_sub_dir(&self) -> u32 {
        self.sub_dir
    }
    fn set_sub_file(&mut self, index: u32) {
        self.sub_file = index;
    }
    fn get_sub_file(&self) -> u32 {
        self.sub_file
    }
    fn set_next(&mut self, index: u32) {
        self.next = index;
    }
    fn get_next(&self) -> u32 {
        self.next
    }
}

impl ParentedKey for SaveExtKey {
    type NameType = [u8; 16];
    fn get_name(&self) -> [u8; 16] {
        self.name
    }
    fn get_parent(&self) -> u32 {
        self.parent
    }
    fn new(parent: u32, name: [u8; 16]) -> SaveExtKey {
        SaveExtKey { parent, name }
    }
}

#[derive(ByteStruct)]
#[byte_struct_le]
pub(crate) struct FsInfo {
    pub unknown: u32,
    pub block_len: u32,
    pub dir_hash_offset: u64,
    pub dir_buckets: u32,
    pub p0: u32,
    pub file_hash_offset: u64,
    pub file_buckets: u32,
    pub p1: u32,
    pub fat_offset: u64,
    pub fat_size: u32,
    pub p2: u32,
    pub data_offset: u64,
    pub data_block_count: u32,
    pub p3: u32,
    pub dir_table: u64,
    pub max_dir: u32,
    pub p4: u32,
    pub file_table: u64,
    pub max_file: u32,
    pub p5: u32,
}

#[allow(unused_variables)]
pub trait FileSystem {
    type CenterType;
    type FileType;
    type DirType;

    fn file_open_ino(center: Rc<Self::CenterType>, ino: u32) -> Result<Self::FileType, Error> {
        make_error(Error::Unsupported)
    }

    fn file_rename(
        file: &mut Self::FileType,
        parent: &Self::DirType,
        name: [u8; 16],
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
        name: [u8; 16],
    ) -> Result<(), Error> {
        make_error(Error::Unsupported)
    }

    fn dir_get_parent_ino(dir: &Self::DirType) -> u32;

    fn dir_get_ino(dir: &Self::DirType) -> u32;

    fn open_sub_dir(dir: &Self::DirType, name: [u8; 16]) -> Result<Self::DirType, Error> {
        make_error(Error::Unsupported)
    }

    fn open_sub_file(dir: &Self::DirType, name: [u8; 16]) -> Result<Self::FileType, Error> {
        make_error(Error::Unsupported)
    }

    fn list_sub_dir(dir: &Self::DirType) -> Result<Vec<([u8; 16], u32)>, Error> {
        make_error(Error::Unsupported)
    }

    fn list_sub_file(dir: &Self::DirType) -> Result<Vec<([u8; 16], u32)>, Error> {
        make_error(Error::Unsupported)
    }

    fn new_sub_dir(dir: &Self::DirType, name: [u8; 16]) -> Result<Self::DirType, Error> {
        make_error(Error::Unsupported)
    }

    fn new_sub_file(
        dir: &Self::DirType,
        name: [u8; 16],
        len: usize,
    ) -> Result<Self::FileType, Error> {
        make_error(Error::Unsupported)
    }

    fn dir_delete(dir: Self::DirType) -> Result<(), Error> {
        make_error(Error::Unsupported)
    }

    fn commit(center: &Self::CenterType) -> Result<(), Error> {
        make_error(Error::Unsupported)
    }
}

#[cfg(test)]
mod test {
    use crate::save_ext_common::*;
    #[test]
    fn struct_size() {
        assert_eq!(FsInfo::BYTE_LEN, 0x68);
    }

}
