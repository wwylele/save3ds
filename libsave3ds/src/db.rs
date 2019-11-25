use crate::diff::Diff;
use crate::error::*;
use crate::fat::*;
use crate::file_system::*;
use crate::fs_meta::{self, DirInfo, FileInfo, FsInfo, ParentedKey};
use crate::misc::*;
use crate::random_access_file::*;
use crate::signed_file::*;
use crate::sub_file::SubFile;
use byte_struct::*;
use std::rc::Rc;

#[derive(ByteStruct, Clone, PartialEq)]
#[byte_struct_le]
struct DbDirKey {
    parent: u32,
}

impl ParentedKey for DbDirKey {
    type NameType = ();
    fn get_name(&self) {}
    fn get_parent(&self) -> u32 {
        self.parent
    }
    fn new(parent: u32, _name: ()) -> DbDirKey {
        DbDirKey { parent }
    }
}

#[derive(ByteStruct, Clone)]
#[byte_struct_le]
struct DbDir {
    next: u32,
    sub_dir: u32,
    sub_file: u32,
    padding: [u8; 12],
}

impl DirInfo for DbDir {
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
    fn new_root() -> Self {
        DbDir {
            next: 0,
            sub_dir: 0,
            sub_file: 0,
            padding: [0; 12],
        }
    }
}

#[derive(ByteStruct, Clone, PartialEq)]
#[byte_struct_le]
pub(crate) struct DbFileKey {
    parent: u32,
    name: u64,
}

impl ParentedKey for DbFileKey {
    type NameType = u64;
    fn get_name(&self) -> u64 {
        self.name
    }
    fn get_parent(&self) -> u32 {
        self.parent
    }
    fn new(parent: u32, name: u64) -> DbFileKey {
        DbFileKey { parent, name }
    }
}

#[derive(ByteStruct, Clone)]
#[byte_struct_le]
struct DbFile {
    next: u32,
    padding1: u32,
    block: u32,
    size: u64,
    padding2: u64,
}

impl FileInfo for DbFile {
    fn set_next(&mut self, index: u32) {
        self.next = index;
    }
    fn get_next(&self) -> u32 {
        self.next
    }
}

type FsMeta = fs_meta::FsMeta<DbDirKey, DbDir, DbFileKey, DbFile>;
type DirMeta = fs_meta::DirMeta<DbDirKey, DbDir, DbFileKey, DbFile>;
type FileMeta = fs_meta::FileMeta<DbDirKey, DbDir, DbFileKey, DbFile>;

#[derive(ByteStruct)]
#[byte_struct_le]
struct DbHeader {
    magic: [u8; 4],
    version: u32,
    fs_info_offset: u64,
    image_size: u64,
    image_block_len: u32,
    padding: u32,
}

#[derive(PartialEq)]
pub enum DbType {
    Ticket,
    NandTitle,
    NandImport,
    TmpTitle,
    TmpImport,
    SdTitle,
    SdImport,
}

struct FakeSizeFile {
    parent: Rc<dyn RandomAccessFile>,
    len: usize,
}

impl RandomAccessFile for FakeSizeFile {
    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error> {
        if pos >= self.parent.len() {
            return Ok(());
        }
        let end = std::cmp::min(pos + buf.len(), self.parent.len());
        self.parent.read(pos, &mut buf[0..end - pos])
    }
    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error> {
        if pos >= self.parent.len() {
            return Ok(());
        }
        let end = std::cmp::min(pos + buf.len(), self.parent.len());
        self.parent.write(pos, &buf[0..end - pos])
    }
    fn len(&self) -> usize {
        self.len
    }
    fn commit(&self) -> Result<(), Error> {
        Ok(())
    }
}

struct DbSigner {
    pub id: u32,
}

impl Signer for DbSigner {
    fn block(&self, mut data: Vec<u8>) -> Vec<u8> {
        let mut result = Vec::from(&b"CTR-9DB0"[..]);
        result.extend(&self.id.to_le_bytes());
        result.append(&mut data);
        result
    }
}

struct DbInner {
    diff: Rc<Diff>,
    fat: Rc<Fat>,
    fs: Rc<FsMeta>,
    block_len: usize,
    block_count: usize,
}

pub struct Db {
    center: Rc<DbInner>,
}

impl Db {
    pub(crate) fn new(
        file: Rc<dyn RandomAccessFile>,
        db_type: DbType,
        key: [u8; 16],
    ) -> Result<Db, Error> {
        let signer: (Box<dyn Signer>, [u8; 16]) = (
            Box::new(DbSigner {
                id: match db_type {
                    DbType::Ticket => 0,
                    DbType::SdTitle | DbType::NandTitle => 2,
                    DbType::SdImport | DbType::NandImport => 3,
                    DbType::TmpTitle => 4,
                    DbType::TmpImport => 5,
                },
            }),
            key,
        );
        let diff = Rc::new(Diff::new(file, Some(signer))?);
        let pre_len = if db_type == DbType::Ticket {
            0x10
        } else {
            0x80
        };

        if db_type == DbType::Ticket {
            let mut magic = [0; 4];
            diff.partition().read(0, &mut magic)?;
            if magic != *b"TICK" {
                return make_error(Error::MagicMismatch);
            }
        } else {
            let mut magic = [0; 8];
            diff.partition().read(0, &mut magic)?;
            if magic
                != match db_type {
                    DbType::NandTitle => *b"NANDTDB\0",
                    DbType::NandImport => *b"NANDIDB\0",
                    DbType::TmpTitle => *b"TEMPIDB\0",
                    DbType::TmpImport => *b"TEMPIDB\0",
                    DbType::SdTitle => *b"TEMPTDB\0",
                    DbType::SdImport => *b"TEMPTDB\0",
                    _ => unreachable!(),
                }
            {
                return make_error(Error::MagicMismatch);
            }
        }

        let without_pre = Rc::new(SubFile::new(
            diff.partition().clone(),
            pre_len,
            diff.partition().len() - pre_len,
        )?);

        let header: DbHeader = read_struct(without_pre.as_ref(), 0)?;
        if header.magic != *b"BDRI" || header.version != 0x30000 {
            return make_error(Error::MagicMismatch);
        }
        let fs_info: FsInfo = read_struct(without_pre.as_ref(), header.fs_info_offset as usize)?;
        if fs_info.data_block_count != fs_info.fat_size {
            return make_error(Error::SizeMismatch);
        }

        let dir_hash = Rc::new(SubFile::new(
            without_pre.clone(),
            fs_info.dir_hash_offset as usize,
            fs_info.dir_buckets as usize * 4,
        )?);

        let file_hash = Rc::new(SubFile::new(
            without_pre.clone(),
            fs_info.file_hash_offset as usize,
            fs_info.file_buckets as usize * 4,
        )?);

        let fat_table = Rc::new(SubFile::new(
            without_pre.clone(),
            fs_info.fat_offset as usize,
            (fs_info.fat_size + 1) as usize * 8,
        )?);

        let data_offset = fs_info.data_offset as usize;
        let data_len = (fs_info.data_block_count * fs_info.block_len) as usize;
        let data_end = data_len + data_offset;
        let data_delta = if without_pre.len() < data_end {
            data_end - without_pre.len()
        } else {
            0
        };

        println!("Database file end fixup: 0x{:x}", data_delta);

        let data: Rc<dyn RandomAccessFile> = Rc::new(FakeSizeFile {
            parent: Rc::new(SubFile::new(
                without_pre.clone(),
                fs_info.data_offset as usize,
                data_len - data_delta,
            )?),
            len: data_len,
        });

        let fat = Fat::new(fat_table, data, fs_info.block_len as usize)?;

        let dir_table: Rc<dyn RandomAccessFile> = Rc::new(FatFile::open(
            fat.clone(),
            fs_info.dir_table.block_index as usize,
        )?);

        let file_table: Rc<dyn RandomAccessFile> = Rc::new(FatFile::open(
            fat.clone(),
            fs_info.file_table.block_index as usize,
        )?);

        let fs = FsMeta::new(dir_hash, dir_table, file_hash, file_table)?;

        Ok(Db {
            center: Rc::new(DbInner {
                diff,
                fat,
                fs,
                block_len: fs_info.block_len as usize,
                block_count: fs_info.data_block_count as usize,
            }),
        })
    }
}

pub struct File {
    center: Rc<DbInner>,
    meta: FileMeta,
    data: Option<FatFile>,
    len: usize,
}

impl File {
    fn from_meta(center: Rc<DbInner>, meta: FileMeta) -> Result<File, Error> {
        let info = meta.get_info()?;
        let len = info.size as usize;
        let data = if info.block == 0x8000_0000 {
            if len != 0 {
                return make_error(Error::SizeMismatch);
            }
            None
        } else {
            let fat_file = FatFile::open(center.fat.clone(), info.block as usize)?;
            if len == 0 || len > fat_file.len() {
                return make_error(Error::SizeMismatch);
            }
            Some(fat_file)
        };
        Ok(File {
            center,
            meta,
            data,
            len,
        })
    }
}

impl FileSystemFile for File {
    type NameType = u64;
    type DirType = Dir;

    fn rename(&mut self, parent: &Self::DirType, name: u64) -> Result<(), Error> {
        if parent.meta.open_sub_file(name).is_ok() {
            return make_error(Error::AlreadyExist);
        }
        self.meta.rename(&parent.meta, name)
    }

    fn get_parent_ino(&self) -> Result<u32, Error> {
        self.meta.get_parent_ino()
    }

    fn get_ino(&self) -> u32 {
        self.meta.get_ino()
    }

    fn delete(self) -> Result<(), Error> {
        if let Some(f) = self.data {
            f.delete()?;
        }
        self.meta.delete()
    }

    fn resize(&mut self, len: usize) -> Result<(), Error> {
        if len == self.len {
            return Ok(());
        }

        self.meta.check_exclusive()?;

        let mut info = self.meta.get_info()?;

        if self.len == 0 {
            // zero => non-zero
            let (fat_file, block) = FatFile::create(
                self.center.fat.clone(),
                divide_up(len, self.center.block_len),
            )?;
            self.data = Some(fat_file);
            info.block = block as u32;
        } else if len == 0 {
            // non-zero => zero
            self.data.take().unwrap().delete()?;
            info.block = 0x8000_0000;
        } else {
            self.data
                .as_mut()
                .unwrap()
                .resize(divide_up(len, self.center.block_len))?;
        }

        info.size = len as u64;
        self.meta.set_info(info)?;

        self.len = len;

        Ok(())
    }

    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error> {
        if pos + buf.len() > self.len {
            return make_error(Error::OutOfBound);
        }
        self.data.as_ref().unwrap().read(pos, buf)
    }

    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error> {
        if pos + buf.len() > self.len {
            return make_error(Error::OutOfBound);
        }
        self.data.as_ref().unwrap().write(pos, buf)
    }

    fn len(&self) -> usize {
        self.len
    }

    fn commit(&self) -> Result<(), Error> {
        Ok(())
    }
}

pub struct Dir {
    center: Rc<DbInner>,
    meta: DirMeta,
}

impl FileSystemDir for Dir {
    type NameType = u64;
    type FileType = File;

    fn get_parent_ino(&self) -> Result<u32, Error> {
        self.meta.get_parent_ino()
    }

    fn get_ino(&self) -> u32 {
        self.meta.get_ino()
    }

    fn open_sub_file(&self, name: u64) -> Result<Self::FileType, Error> {
        File::from_meta(self.center.clone(), self.meta.open_sub_file(name)?)
    }

    fn list_sub_dir(&self) -> Result<Vec<(u64, u32)>, Error> {
        Ok(vec![])
    }

    fn list_sub_file(&self) -> Result<Vec<(u64, u32)>, Error> {
        self.meta.list_sub_file()
    }

    fn new_sub_file(&self, name: u64, len: usize) -> Result<Self::FileType, Error> {
        if self.open_sub_file(name).is_ok() || self.open_sub_dir(name).is_ok() {
            return make_error(Error::AlreadyExist);
        }
        let (fat_file, block) = if len == 0 {
            (None, 0x8000_0000)
        } else {
            let (fat_file, block) = FatFile::create(
                self.center.fat.clone(),
                divide_up(len, self.center.block_len),
            )?;
            (Some(fat_file), block as u32)
        };
        match self.meta.new_sub_file(
            name,
            DbFile {
                next: 0,
                padding1: 0,
                block,
                size: len as u64,
                padding2: 0,
            },
        ) {
            Err(e) => {
                if let Some(f) = fat_file {
                    f.delete()?;
                }
                Err(e)
            }
            Ok(meta) => File::from_meta(self.center.clone(), meta),
        }
    }

    fn rename(&mut self, _parent: &Self, _name: Self::NameType) -> Result<(), Error> {
        make_error(Error::Unsupported)
    }

    fn open_sub_dir(&self, _name: Self::NameType) -> Result<Self, Error> {
        make_error(Error::NotFound)
    }

    fn new_sub_dir(&self, _name: Self::NameType) -> Result<Self, Error> {
        make_error(Error::Unsupported)
    }

    fn delete(self) -> Result<(), Error> {
        make_error(Error::DeletingRoot)
    }
}

impl FileSystem for Db {
    type FileType = File;
    type DirType = Dir;
    type NameType = u64;

    fn open_file(&self, ino: u32) -> Result<Self::FileType, Error> {
        let meta = FileMeta::open_ino(self.center.fs.clone(), ino)?;
        File::from_meta(self.center.clone(), meta)
    }

    fn open_dir(&self, ino: u32) -> Result<Self::DirType, Error> {
        let meta = DirMeta::open_ino(self.center.fs.clone(), ino)?;
        Ok(Dir {
            center: self.center.clone(),
            meta,
        })
    }

    fn commit(&self) -> Result<(), Error> {
        self.center.diff.commit()
    }

    fn stat(&self) -> Result<Stat, Error> {
        let meta_stat = self.center.fs.stat()?;
        Ok(Stat {
            block_len: self.center.block_len,
            total_blocks: self.center.block_count,
            free_blocks: self.center.fat.free_blocks(),
            total_files: meta_stat.files.total,
            free_files: meta_stat.files.free,
            total_dirs: meta_stat.dirs.total,
            free_dirs: meta_stat.dirs.free,
        })
    }
}
