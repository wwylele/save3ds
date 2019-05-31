use crate::difi_partition::*;
use crate::disa::Disa;
use crate::error::*;
use crate::fat::*;
use crate::file_system::*;
use crate::fs_meta::{self, FileInfo, FsInfo};
use crate::memory_file::MemoryFile;
use crate::misc::*;
use crate::random_access_file::*;
use crate::save_ext_common::*;
use crate::signed_file::*;
use crate::sub_file::SubFile;
use byte_struct::*;
use std::rc::Rc;

#[derive(ByteStruct, Clone)]
#[byte_struct_le]
pub struct SaveFile {
    pub next: u32,
    pub padding1: u32,
    pub block: u32,
    pub size: u64,
    pub padding2: u32,
}

impl FileInfo for SaveFile {
    fn set_next(&mut self, index: u32) {
        self.next = index;
    }
    fn get_next(&self) -> u32 {
        self.next
    }
}

type FsMeta = fs_meta::FsMeta<SaveExtKey, SaveExtDir, SaveExtKey, SaveFile>;
type DirMeta = fs_meta::DirMeta<SaveExtKey, SaveExtDir, SaveExtKey, SaveFile>;
type FileMeta = fs_meta::FileMeta<SaveExtKey, SaveExtDir, SaveExtKey, SaveFile>;

pub struct NandSaveSigner {
    pub id: u32,
}

impl Signer for NandSaveSigner {
    fn block(&self, mut data: Vec<u8>) -> Vec<u8> {
        let mut result = Vec::from(&b"CTR-SYS0"[..]);
        result.extend(&self.id.to_le_bytes());
        result.extend(&[0; 4]);
        result.append(&mut data);
        result
    }
}

pub struct CtrSav0Signer {}

impl Signer for CtrSav0Signer {
    fn block(&self, mut data: Vec<u8>) -> Vec<u8> {
        let mut result = Vec::from(&b"CTR-SAV0"[..]);
        result.append(&mut data);
        result
    }
}

pub struct SdSaveSigner {
    pub id: u64,
}
impl Signer for SdSaveSigner {
    fn block(&self, data: Vec<u8>) -> Vec<u8> {
        let mut result = Vec::from(&b"CTR-SIGN"[..]);
        result.extend(&self.id.to_le_bytes());
        result.append(&mut CtrSav0Signer {}.hash(data));
        result
    }
}

#[derive(ByteStruct)]
#[byte_struct_le]
struct SaveHeader {
    magic: [u8; 4],
    version: u32,
    fs_info_offset: u64,
    image_size: u64,
    image_block_len: u32,
    padding: u32,
}

pub struct SaveData {
    disa: Rc<Disa>,
    fat: Rc<Fat>,
    fs: Rc<FsMeta>,
    block_len: usize,
}

#[derive(Clone)]
pub enum SaveDataType {
    Nand([u8; 16], u32),
    Sd([u8; 16], u64),
    Bare,
}

pub enum SaveDataBlockType {
    Small, // 512-byte block, for game save
    Large, // 4096-byte block, for NAND (system) save
}

pub struct SaveDataFormatParam {
    pub block_type: SaveDataBlockType,
    pub max_dir: usize,
    pub dir_buckets: usize,
    pub max_file: usize,
    pub file_buckets: usize,
    pub duplicate_data: bool,
}

struct SaveDataInfo {
    block_len: usize,
    param_a: DifiPartitionParam,
    param_b: Option<DifiPartitionParam>,
    dir_hash_offset: usize,
    file_hash_offset: usize,
    fat_offset: usize,
    data_block_count: usize,
    data_offset: Option<usize>,
    dir_table_offset: Option<usize>,
    file_table_offset: Option<usize>,
}

impl SaveData {
    fn calculate_info(param: &SaveDataFormatParam, block_count: usize) -> SaveDataInfo {
        let block_len = match param.block_type {
            SaveDataBlockType::Small => 512,
            SaveDataBlockType::Large => 4096,
        };
        let fs_info_offset = SaveHeader::BYTE_LEN;
        let dir_hash_offset = fs_info_offset + FsInfo::BYTE_LEN;
        let file_hash_offset = dir_hash_offset + param.dir_buckets * 4;
        let fat_offset = file_hash_offset + param.file_buckets * 4;

        let dir_table_len = (param.max_dir + 2) * (SaveExtKey::BYTE_LEN + SaveExtDir::BYTE_LEN + 4);
        let file_table_len = (param.max_file + 1) * (SaveExtKey::BYTE_LEN + SaveFile::BYTE_LEN + 4);

        if param.duplicate_data {
            let data_len = align_up(dir_table_len, block_len)
                + align_up(file_table_len, block_len)
                + block_count * block_len;
            let data_block_count = data_len / block_len;
            let fat_len = (data_block_count + 1) * 8;
            let data_offset = align_up(fat_offset + fat_len, block_len);
            let inner_a_len = data_offset + data_len;

            let param_a = DifiPartitionParam {
                dpfs_level2_block_len: 128,
                dpfs_level3_block_len: 4096,
                ivfc_level1_block_len: 512,
                ivfc_level2_block_len: 512,
                ivfc_level3_block_len: 4096,
                ivfc_level4_block_len: 4096,
                data_len: inner_a_len,
                external_ivfc_level4: false,
            };
            let param_b = None;

            SaveDataInfo {
                block_len,
                param_a,
                param_b,
                dir_hash_offset,
                file_hash_offset,
                fat_offset,
                data_block_count,
                data_offset: Some(data_offset),
                dir_table_offset: None,
                file_table_offset: None,
            }
        } else {
            let data_block_count = block_count;
            let fat_len = (data_block_count + 1) * 8;
            let dir_table_offset = fat_offset + fat_len;
            let file_table_offset = dir_table_offset + dir_table_len;
            let inner_a_len = align_up(file_table_offset + file_table_len, block_len);
            let inner_b_len = block_count * block_len;

            let param_a = DifiPartitionParam {
                dpfs_level2_block_len: 128,
                dpfs_level3_block_len: 4096,
                ivfc_level1_block_len: 512,
                ivfc_level2_block_len: 512,
                ivfc_level3_block_len: 4096,
                ivfc_level4_block_len: block_len,
                data_len: inner_a_len,
                external_ivfc_level4: false,
            };

            let param_b = Some(DifiPartitionParam {
                dpfs_level2_block_len: 128,
                dpfs_level3_block_len: 4096,
                ivfc_level1_block_len: 512,
                ivfc_level2_block_len: 512,
                ivfc_level3_block_len: 4096,
                ivfc_level4_block_len: block_len,
                data_len: inner_b_len,
                external_ivfc_level4: true,
            });

            SaveDataInfo {
                block_len,
                param_a,
                param_b,
                dir_hash_offset,
                file_hash_offset,
                fat_offset,
                data_block_count,
                data_offset: None,
                dir_table_offset: Some(dir_table_offset),
                file_table_offset: Some(file_table_offset),
            }
        }
    }

    pub fn calculate_size(param: &SaveDataFormatParam, block_count: usize) -> usize {
        let info = SaveData::calculate_info(param, block_count);
        Disa::calculate_size(&info.param_a, info.param_b.as_ref())
    }

    pub fn calculate_capacity(param: &SaveDataFormatParam, disa_len: usize) -> usize {
        let min_disa_len = SaveData::calculate_size(param, 1);
        if min_disa_len > disa_len {
            return 0;
        }
        let block_len = match param.block_type {
            SaveDataBlockType::Small => 512,
            SaveDataBlockType::Large => 4096,
        };
        let mut min_block = 1;
        let mut max_block = disa_len / block_len + 1;
        while max_block - min_block > 1 {
            let mid_block = (min_block + max_block) / 2;
            let required_len = SaveData::calculate_size(param, mid_block);
            if required_len > disa_len {
                max_block = mid_block;
            } else {
                min_block = mid_block;
            }
        }
        min_block
    }

    pub fn from_vec(v: Vec<u8>, save_data_type: SaveDataType) -> Result<Rc<SaveData>, Error> {
        let file = Rc::new(MemoryFile::new(v));
        SaveData::new(file, save_data_type)
    }

    fn get_signer(save_data_type: SaveDataType) -> Option<(Box<Signer>, [u8; 16])> {
        match save_data_type {
            SaveDataType::Bare => None,
            SaveDataType::Nand(key, id) => Some((Box::new(NandSaveSigner { id }), key)),
            SaveDataType::Sd(key, id) => Some((Box::new(SdSaveSigner { id }), key)),
        }
    }

    pub fn format(
        file: Rc<RandomAccessFile>,
        save_data_type: SaveDataType,
        param: &SaveDataFormatParam,
        block_count: usize,
    ) -> Result<(), Error> {
        let info = SaveData::calculate_info(param, block_count);
        Disa::format(
            file.clone(),
            SaveData::get_signer(save_data_type.clone()),
            &info.param_a,
            info.param_b.as_ref(),
        )?;

        let disa = Rc::new(Disa::new(file, SaveData::get_signer(save_data_type))?);

        let dir_hash = Rc::new(SubFile::new(
            disa[0].clone(),
            info.dir_hash_offset,
            param.dir_buckets * 4,
        )?);

        let file_hash = Rc::new(SubFile::new(
            disa[0].clone(),
            info.file_hash_offset,
            param.file_buckets * 4,
        )?);

        let fat_table = Rc::new(SubFile::new(
            disa[0].clone(),
            info.fat_offset,
            (info.data_block_count + 1) * 8,
        )?);

        Fat::format(fat_table.as_ref())?;

        let data: Rc<RandomAccessFile> = if disa.partition_count() == 2 {
            disa[1].clone()
        } else {
            Rc::new(SubFile::new(
                disa[0].clone(),
                info.data_offset.unwrap(),
                info.data_block_count * info.block_len,
            )?)
        };

        let dir_table_len = (param.max_dir + 2) * (SaveExtKey::BYTE_LEN + SaveExtDir::BYTE_LEN + 4);
        let file_table_len = (param.max_file + 1) * (SaveExtKey::BYTE_LEN + SaveFile::BYTE_LEN + 4);

        let (dir_table, file_table) = if disa.partition_count() == 2 {
            let dir_table = Rc::new(SubFile::new(
                disa[0].clone(),
                info.dir_table_offset.unwrap(),
                dir_table_len,
            )?);
            let file_table = Rc::new(SubFile::new(
                disa[0].clone(),
                info.file_table_offset.unwrap(),
                file_table_len,
            )?);
            FsMeta::format(
                dir_hash,
                dir_table,
                param.max_dir + 2,
                file_hash,
                file_table,
                param.max_file + 1,
            )?;
            let dir_table_combo = info.dir_table_offset.unwrap() as u64;
            let file_table_combo = info.file_table_offset.unwrap() as u64;
            (dir_table_combo, file_table_combo)
        } else {
            let fat = Fat::new(fat_table, data, info.block_len)?;
            let (dir_table, dir_table_block_index) =
                FatFile::create(fat.clone(), 1 + (dir_table_len - 1) / info.block_len)?;
            let (file_table, file_table_block_index) =
                FatFile::create(fat.clone(), 1 + (file_table_len - 1) / info.block_len)?;
            let dir_table_combo = (dir_table_block_index as u64)
                | (((dir_table.len() / info.block_len) as u64) << 32);
            let file_table_combo = (file_table_block_index as u64)
                | (((file_table.len() / info.block_len) as u64) << 32);
            FsMeta::format(
                dir_hash,
                Rc::new(dir_table),
                param.max_dir + 2,
                file_hash,
                Rc::new(file_table),
                param.max_file + 1,
            )?;
            (dir_table_combo, file_table_combo)
        };

        let header = SaveHeader {
            magic: *b"SAVE",
            version: 0x40000,
            fs_info_offset: SaveHeader::BYTE_LEN as u64,
            image_size: (disa[0].len() / info.block_len) as u64,
            image_block_len: info.block_len as u32,
            padding: 0,
        };

        write_struct(disa[0].as_ref(), 0, header)?;

        let fs_info = FsInfo {
            unknown: 0,
            block_len: info.block_len as u32,
            dir_hash_offset: info.dir_hash_offset as u64,
            dir_buckets: param.dir_buckets as u32,
            p0: 0,
            file_hash_offset: info.file_hash_offset as u64,
            file_buckets: param.file_buckets as u32,
            p1: 0,
            fat_offset: info.fat_offset as u64,
            fat_size: info.data_block_count as u32,
            p2: 0,
            data_offset: info.data_offset.unwrap_or(0) as u64,
            data_block_count: info.data_block_count as u32,
            p3: 0,
            dir_table,
            max_dir: param.max_dir as u32,
            p4: 0,
            file_table,
            max_file: param.max_file as u32,
            p5: 0,
        };

        write_struct(disa[0].as_ref(), SaveHeader::BYTE_LEN, fs_info)?;
        disa.commit()?;
        Ok(())
    }

    pub fn new(
        file: Rc<RandomAccessFile>,
        save_data_type: SaveDataType,
    ) -> Result<Rc<SaveData>, Error> {
        let disa = Rc::new(Disa::new(file, SaveData::get_signer(save_data_type))?);
        let header: SaveHeader = read_struct(disa[0].as_ref(), 0)?;
        if header.magic != *b"SAVE" || header.version != 0x40000 {
            return make_error(Error::MagicMismatch);
        }
        let fs_info: FsInfo = read_struct(disa[0].as_ref(), header.fs_info_offset as usize)?;
        if fs_info.data_block_count != fs_info.fat_size {
            return make_error(Error::SizeMismatch);
        }

        let dir_hash = Rc::new(SubFile::new(
            disa[0].clone(),
            fs_info.dir_hash_offset as usize,
            fs_info.dir_buckets as usize * 4,
        )?);

        let file_hash = Rc::new(SubFile::new(
            disa[0].clone(),
            fs_info.file_hash_offset as usize,
            fs_info.file_buckets as usize * 4,
        )?);

        let fat_table = Rc::new(SubFile::new(
            disa[0].clone(),
            fs_info.fat_offset as usize,
            (fs_info.fat_size + 1) as usize * 8,
        )?);

        let data: Rc<RandomAccessFile> = if disa.partition_count() == 2 {
            disa[1].clone()
        } else {
            Rc::new(SubFile::new(
                disa[0].clone(),
                fs_info.data_offset as usize,
                (fs_info.data_block_count * fs_info.block_len) as usize,
            )?)
        };

        let fat = Fat::new(fat_table, data, fs_info.block_len as usize)?;

        let dir_table: Rc<RandomAccessFile> = if disa.partition_count() == 2 {
            Rc::new(SubFile::new(
                disa[0].clone(),
                fs_info.dir_table as usize,
                (fs_info.max_dir + 2) as usize * (SaveExtKey::BYTE_LEN + SaveExtDir::BYTE_LEN + 4),
            )?)
        } else {
            let block = (fs_info.dir_table & 0xFFFF_FFFF) as usize;
            Rc::new(FatFile::open(fat.clone(), block)?)
        };

        let file_table: Rc<RandomAccessFile> = if disa.partition_count() == 2 {
            Rc::new(SubFile::new(
                disa[0].clone(),
                fs_info.file_table as usize,
                (fs_info.max_file + 1) as usize * (SaveExtKey::BYTE_LEN + SaveFile::BYTE_LEN + 4),
            )?)
        } else {
            let block = (fs_info.file_table & 0xFFFF_FFFF) as usize;
            Rc::new(FatFile::open(fat.clone(), block)?)
        };

        let fs = FsMeta::new(dir_hash, dir_table, file_hash, file_table)?;

        Ok(Rc::new(SaveData {
            disa,
            fat,
            fs,
            block_len: fs_info.block_len as usize,
        }))
    }
}

pub struct File {
    center: Rc<SaveData>,
    meta: FileMeta,
    data: Option<FatFile>,
    len: usize,
}

impl File {
    fn from_meta(center: Rc<SaveData>, meta: FileMeta) -> Result<File, Error> {
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

pub struct Dir {
    center: Rc<SaveData>,
    meta: DirMeta,
}

pub struct SaveDataFileSystem {}
impl FileSystem for SaveDataFileSystem {
    type CenterType = SaveData;
    type FileType = File;
    type DirType = Dir;
    type NameType = [u8; 16];

    fn file_open_ino(center: Rc<Self::CenterType>, ino: u32) -> Result<Self::FileType, Error> {
        let meta = FileMeta::open_ino(center.fs.clone(), ino)?;
        File::from_meta(center, meta)
    }

    fn file_rename(
        file: &mut Self::FileType,
        parent: &Self::DirType,
        name: [u8; 16],
    ) -> Result<(), Error> {
        file.meta.rename(&parent.meta, name)
    }

    fn file_get_parent_ino(file: &Self::FileType) -> u32 {
        file.meta.get_parent_ino()
    }

    fn file_get_ino(file: &Self::FileType) -> u32 {
        file.meta.get_ino()
    }

    fn file_delete(file: Self::FileType) -> Result<(), Error> {
        if let Some(f) = file.data {
            f.delete()?;
        }
        file.meta.delete()
    }

    fn resize(file: &mut Self::FileType, len: usize) -> Result<(), Error> {
        if len == file.len {
            return Ok(());
        }

        let mut info = file.meta.get_info()?;

        if file.len == 0 {
            // zero => non-zero
            let (fat_file, block) = FatFile::create(
                file.center.fat.clone(),
                1 + (len - 1) / file.center.block_len,
            )?;
            file.data = Some(fat_file);
            info.block = block as u32;
        } else if len == 0 {
            // non-zero => zero
            file.data.take().unwrap().delete()?;
            info.block = 0x8000_0000;
        } else {
            file.data
                .as_mut()
                .unwrap()
                .resize(1 + (len - 1) / file.center.block_len)?;
        }

        info.size = len as u64;
        file.meta.set_info(info)?;

        file.len = len;

        Ok(())
    }

    fn read(file: &Self::FileType, pos: usize, buf: &mut [u8]) -> Result<(), Error> {
        if buf.is_empty() {
            return Ok(());
        }
        if pos + buf.len() > file.len {
            return make_error(Error::OutOfBound);
        }
        file.data.as_ref().unwrap().read(pos, buf)
    }

    fn write(file: &Self::FileType, pos: usize, buf: &[u8]) -> Result<(), Error> {
        if buf.is_empty() {
            return Ok(());
        }
        if pos + buf.len() > file.len {
            return make_error(Error::OutOfBound);
        }
        file.data.as_ref().unwrap().write(pos, buf)
    }

    fn len(file: &Self::FileType) -> usize {
        file.len
    }

    fn open_root(center: Rc<Self::CenterType>) -> Result<Self::DirType, Error> {
        let meta = DirMeta::open_root(center.fs.clone())?;
        Ok(Dir { center, meta })
    }

    fn dir_open_ino(center: Rc<Self::CenterType>, ino: u32) -> Result<Self::DirType, Error> {
        let meta = DirMeta::open_ino(center.fs.clone(), ino)?;
        Ok(Dir { center, meta })
    }

    fn dir_rename(
        dir: &mut Self::DirType,
        parent: &Self::DirType,
        name: [u8; 16],
    ) -> Result<(), Error> {
        if Self::open_sub_file(&parent, name).is_ok() || Self::open_sub_dir(&parent, name).is_ok() {
            return make_error(Error::AlreadyExist);
        }
        dir.meta.rename(&parent.meta, name)
    }

    fn dir_get_parent_ino(dir: &Self::DirType) -> u32 {
        dir.meta.get_parent_ino()
    }

    fn dir_get_ino(dir: &Self::DirType) -> u32 {
        dir.meta.get_ino()
    }

    fn open_sub_dir(dir: &Self::DirType, name: [u8; 16]) -> Result<Self::DirType, Error> {
        Ok(Dir {
            center: dir.center.clone(),
            meta: dir.meta.open_sub_dir(name)?,
        })
    }

    fn open_sub_file(dir: &Self::DirType, name: [u8; 16]) -> Result<Self::FileType, Error> {
        File::from_meta(dir.center.clone(), dir.meta.open_sub_file(name)?)
    }

    fn list_sub_dir(dir: &Self::DirType) -> Result<Vec<([u8; 16], u32)>, Error> {
        dir.meta.list_sub_dir()
    }

    fn list_sub_file(dir: &Self::DirType) -> Result<Vec<([u8; 16], u32)>, Error> {
        dir.meta.list_sub_file()
    }

    fn new_sub_dir(dir: &Self::DirType, name: [u8; 16]) -> Result<Self::DirType, Error> {
        if Self::open_sub_file(dir, name).is_ok() || Self::open_sub_dir(dir, name).is_ok() {
            return make_error(Error::AlreadyExist);
        }
        let dir_info = SaveExtDir {
            next: 0,
            sub_dir: 0,
            sub_file: 0,
            padding: 0,
        };
        Ok(Dir {
            center: dir.center.clone(),
            meta: dir.meta.new_sub_dir(name, dir_info)?,
        })
    }

    fn new_sub_file(
        dir: &Self::DirType,
        name: [u8; 16],
        len: usize,
    ) -> Result<Self::FileType, Error> {
        if Self::open_sub_file(dir, name).is_ok() || Self::open_sub_dir(dir, name).is_ok() {
            return make_error(Error::AlreadyExist);
        }
        let (fat_file, block) = if len == 0 {
            (None, 0x8000_0000)
        } else {
            let (fat_file, block) =
                FatFile::create(dir.center.fat.clone(), 1 + (len - 1) / dir.center.block_len)?;
            (Some(fat_file), block as u32)
        };
        match dir.meta.new_sub_file(
            name,
            SaveFile {
                next: 0,
                padding1: 0,
                block: block,
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
            Ok(meta) => File::from_meta(dir.center.clone(), meta),
        }
    }

    fn dir_delete(dir: Self::DirType) -> Result<(), Error> {
        dir.meta.delete()
    }

    fn commit(center: &Self::CenterType) -> Result<(), Error> {
        center.disa.commit()
    }
}

#[cfg(test)]
mod test {
    use crate::save_data::*;
    #[test]
    fn struct_size() {
        assert_eq!(SaveHeader::BYTE_LEN, 0x20);
        assert_eq!(SaveFile::BYTE_LEN, 24);
    }

}
