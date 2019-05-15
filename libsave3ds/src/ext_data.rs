use crate::diff::Diff;
use crate::error::*;
use crate::fat::*;
use crate::fs::{self, FileInfo};
use crate::memory_file::MemoryFile;
use crate::random_access_file::*;
use crate::save_ext_common::*;
use crate::sd::Sd;
use crate::signed_file::*;
use crate::sub_file::SubFile;
use byte_struct::*;
use std::rc::Rc;

#[derive(ByteStruct, Clone)]
#[byte_struct_le]
pub struct ExtFile {
    pub next: u32,
    pub padding1: u32,
    pub block: u32,
    pub unique_id: u64,
    pub padding2: u32,
}

impl FileInfo for ExtFile {
    fn set_next(&mut self, index: u32) {
        self.next = index;
    }
    fn get_next(&self) -> u32 {
        self.next
    }
}

type FsMeta = fs::FsMeta<SaveExtKey, SaveExtDir, SaveExtKey, ExtFile>;
type DirMeta = fs::DirMeta<SaveExtKey, SaveExtDir, SaveExtKey, ExtFile>;
type FileMeta = fs::FileMeta<SaveExtKey, SaveExtDir, SaveExtKey, ExtFile>;

#[derive(ByteStruct)]
#[byte_struct_le]
struct ExtHeader {
    magic: [u8; 4],
    version: u32,
    fs_info_offset: u64,
    image_size: u64,
    image_block_len: u32,
    padding: u32,
    unknown: u64,
    action: u32,
    unknown2: u32,
    mount_id: u32,
    unknown3: u32,
    mount_path: [[u8; 0x10]; 0x10],
}

pub struct ExtData {
    sd: Rc<Sd>,
    id: u64,
    fs: Rc<FsMeta>,
}

impl ExtData {
    pub fn new(sd: Rc<Sd>, id: u64) -> Result<ExtData, Error> {
        let id_high = format!("{:08x}", id >> 32);
        let id_low = format!("{:08x}", id & 0xFFFF_FFFF);
        let meta_path = ["extdata", &id_high, &id_low, "00000000", "00000001"];
        let meta_file = Diff::new(Rc::new(sd.open(&meta_path)?), None)?;

        let header: ExtHeader = read_struct(meta_file.partition().as_ref(), 0)?;
        if header.magic != *b"VSXE" || header.version != 0x30000 {
            return make_error(Error::MagicMismatch);
        }
        let fs_info: FsInfo = read_struct(
            meta_file.partition().as_ref(),
            header.fs_info_offset as usize,
        )?;
        if fs_info.data_block_count != fs_info.fat_size {
            return make_error(Error::SizeMismatch);
        }

        let dir_hash = Rc::new(SubFile::new(
            meta_file.partition().clone(),
            fs_info.dir_hash_offset as usize,
            fs_info.dir_buckets as usize * 4,
        )?);

        let file_hash = Rc::new(SubFile::new(
            meta_file.partition().clone(),
            fs_info.file_hash_offset as usize,
            fs_info.file_buckets as usize * 4,
        )?);

        let fat_table = Rc::new(SubFile::new(
            meta_file.partition().clone(),
            fs_info.fat_offset as usize,
            (fs_info.fat_size + 1) as usize * 8,
        )?);

        let data: Rc<RandomAccessFile> = Rc::new(SubFile::new(
            meta_file.partition().clone(),
            fs_info.data_offset as usize,
            (fs_info.data_block_count * fs_info.block_len) as usize,
        )?);

        let fat = Fat::new(fat_table, data, fs_info.block_len as usize)?;

        let dir_table: Rc<RandomAccessFile> = Rc::new(FatFile::open(
            fat.clone(),
            (fs_info.dir_table & 0xFFFF_FFFF) as usize,
        )?);

        let file_table: Rc<RandomAccessFile> = Rc::new(FatFile::open(
            fat.clone(),
            (fs_info.file_table & 0xFFFF_FFFF) as usize,
        )?);

        let fs = FsMeta::new(dir_hash, dir_table, file_hash, file_table)?;

        Ok(ExtData { sd, id, fs })
    }
}

pub struct File {
    center: Rc<ExtData>,
    meta: FileMeta,
    data: Diff,
}

impl File {
    fn from_meta(center: Rc<ExtData>, meta: FileMeta) -> Result<File, Error> {
        let file_index = meta.get_ino() + 1;
        let id_high = format!("{:08x}", center.id >> 32);
        let id_low = format!("{:08x}", center.id & 0xFFFF_FFFF);
        let fid_high = format!("{:08x}", file_index / 126);
        let fid_low = format!("{:08x}", file_index % 126);
        let path = ["extdata", &id_high, &id_low, &fid_high, &fid_low];
        let data = Diff::new(Rc::new(center.sd.open(&path)?), None)?;

        let info = meta.get_info()?;
        Ok(File { center, meta, data })
    }

    pub fn open_ino(center: Rc<ExtData>, ino: u32) -> Result<File, Error> {
        let meta = FileMeta::open_ino(center.fs.clone(), ino)?;
        File::from_meta(center, meta)
    }

    pub fn rename(&mut self, parent: &Dir, name: [u8; 16]) -> Result<(), Error> {
        if parent.open_sub_file(name).is_ok() || parent.open_sub_dir(name).is_ok() {
            return make_error(Error::AlreadyExist);
        }
        self.meta.rename(&parent.meta, name)
    }

    pub fn get_parent_ino(&self) -> u32 {
        self.meta.get_parent_ino()
    }

    pub fn get_ino(&self) -> u32 {
        self.meta.get_ino()
    }

    pub fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error> {
        self.data.partition().read(pos, buf)
    }

    pub fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error> {
        self.data.partition().write(pos, buf)
    }

    pub fn len(&self) -> usize {
        self.data.partition().len()
    }

    pub fn commit(&self) {
        self.data.commit();
    }
}

pub struct Dir {
    center: Rc<ExtData>,
    meta: DirMeta,
}

impl Dir {
    pub fn open_root(center: Rc<ExtData>) -> Result<Dir, Error> {
        let meta = DirMeta::open_root(center.fs.clone())?;
        Ok(Dir { center, meta })
    }

    pub fn open_ino(center: Rc<ExtData>, ino: u32) -> Result<Dir, Error> {
        let meta = DirMeta::open_ino(center.fs.clone(), ino)?;
        Ok(Dir { center, meta })
    }

    pub fn rename(&mut self, parent: &Dir, name: [u8; 16]) -> Result<(), Error> {
        if parent.open_sub_file(name).is_ok() || parent.open_sub_dir(name).is_ok() {
            return make_error(Error::AlreadyExist);
        }
        self.meta.rename(&parent.meta, name)
    }

    pub fn get_parent_ino(&self) -> u32 {
        self.meta.get_parent_ino()
    }

    pub fn get_ino(&self) -> u32 {
        self.meta.get_ino()
    }

    pub fn open_sub_dir(&self, name: [u8; 16]) -> Result<Dir, Error> {
        Ok(Dir {
            center: self.center.clone(),
            meta: self.meta.open_sub_dir(name)?,
        })
    }

    pub fn open_sub_file(&self, name: [u8; 16]) -> Result<File, Error> {
        File::from_meta(self.center.clone(), self.meta.open_sub_file(name)?)
    }

    pub fn list_sub_dir(&self) -> Result<Vec<([u8; 16], u32)>, Error> {
        self.meta.list_sub_dir()
    }

    pub fn list_sub_file(&self) -> Result<Vec<([u8; 16], u32)>, Error> {
        self.meta.list_sub_file()
    }

    pub fn new_sub_dir(&self, name: [u8; 16]) -> Result<Dir, Error> {
        if self.open_sub_file(name).is_ok() || self.open_sub_dir(name).is_ok() {
            return make_error(Error::AlreadyExist);
        }
        let dir_info = SaveExtDir {
            next: 0,
            sub_dir: 0,
            sub_file: 0,
            padding: 0,
        };
        Ok(Dir {
            center: self.center.clone(),
            meta: self.meta.new_sub_dir(name, dir_info)?,
        })
    }

    pub fn delete(self) -> Result<(), Error> {
        self.meta.delete()
    }
}

#[cfg(test)]
mod test {
    use crate::ext_data::*;
    #[test]
    fn struct_size() {
        assert_eq!(ExtHeader::BYTE_LEN, 0x138);
        assert_eq!(ExtFile::BYTE_LEN, 24);
    }

}
