use crate::diff::Diff;
use crate::difi_partition::DifiPartitionParam;
use crate::error::*;
use crate::fat::*;
use crate::file_system::*;
use crate::fs_meta::{self, FileInfo, FsInfo, OffsetOrFatFile};
use crate::misc::*;
use crate::random_access_file::*;
use crate::save_ext_common::*;
use crate::sd_nand_common::*;
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

type FsMeta = fs_meta::FsMeta<SaveExtKey, SaveExtDir, SaveExtKey, ExtFile>;
type DirMeta = fs_meta::DirMeta<SaveExtKey, SaveExtDir, SaveExtKey, ExtFile>;
type FileMeta = fs_meta::FileMeta<SaveExtKey, SaveExtDir, SaveExtKey, ExtFile>;

#[derive(Clone)]
pub struct ExtSigner {
    pub id: u64,
    pub sub_id: Option<u64>,
}

impl Signer for ExtSigner {
    fn block(&self, mut data: Vec<u8>) -> Vec<u8> {
        let mut result = Vec::from(&b"CTR-EXT0"[..]);
        result.extend(&self.id.to_le_bytes());
        result.extend(&(if self.sub_id.is_some() { 1u32 } else { 0u32 }).to_le_bytes());
        result.extend(&self.sub_id.unwrap_or(0).to_le_bytes());
        result.append(&mut data);
        result
    }
}

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

#[derive(ByteStruct, Debug)]
#[byte_struct_le]
struct Quota {
    magic: [u8; 4],
    version: u32,
    block_len: u32,
    dir_capacity: u32,
    p0: u32,
    max_block: u32,
    p1: u32,
    free_block: u32,
    p2: u32,
    p3: u32,
    potential_free_block: u32,
    p4: u32,
    mount_id: u32,
    p5: u32,
    p6: u32,
    p7: u32,
    mount_len: u64,
}

pub struct ExtDataFormatParam {
    pub max_dir: usize,
    pub dir_buckets: usize,
    pub max_file: usize,
    pub file_buckets: usize,
}

struct ExtDataInner {
    sd_nand: Rc<SdNandFileSystem>,
    base_path: Vec<String>,
    id: u64,
    fs: Rc<FsMeta>,
    meta_file: Diff,
    quota_file: Option<Diff>,
    key: [u8; 16],
    write: bool,
}

pub struct ExtData {
    center: Rc<ExtDataInner>,
}

impl ExtData {
    pub fn format(
        sd_nand: &SdNandFileSystem,
        base_path: &[&str],
        id: u64,
        key: [u8; 16],
        quota: Option<u32>,
        param: &ExtDataFormatParam,
    ) -> Result<(), Error> {
        let id_high = format!("{:08x}", id >> 32);
        let id_low = format!("{:08x}", id & 0xFFFF_FFFF);
        let ext_path: Vec<&str> = base_path
            .iter()
            .cloned()
            .chain([id_high.as_str(), id_low.as_str()].iter().cloned())
            .collect();

        sd_nand.remove_dir(&ext_path)?;

        let mut meta_path = ext_path.clone();
        meta_path.push("00000000");
        meta_path.push("00000001");

        let block_len = 4096;

        let fs_info_offset = ExtHeader::BYTE_LEN;
        let dir_hash_offset = fs_info_offset + FsInfo::BYTE_LEN;
        let file_hash_offset = dir_hash_offset + param.dir_buckets * 4;
        let fat_offset = file_hash_offset + param.file_buckets * 4;

        let dir_table_len = (param.max_dir + 2) * (SaveExtKey::BYTE_LEN + SaveExtDir::BYTE_LEN + 4);
        let file_table_len = (param.max_file + 1) * (SaveExtKey::BYTE_LEN + ExtFile::BYTE_LEN + 4);
        let data_len = align_up(dir_table_len, block_len) + align_up(file_table_len, block_len);
        let data_block_count = data_len / block_len;
        let fat_len = (data_block_count + 1) * 8;
        let data_offset = align_up(fat_offset + fat_len, block_len);
        let partition_end = data_offset + data_len;

        let diff_param = DifiPartitionParam {
            dpfs_level2_block_len: 128,
            dpfs_level3_block_len: 4096,
            ivfc_level1_block_len: 512,
            ivfc_level2_block_len: 512,
            ivfc_level3_block_len: 4096,
            ivfc_level4_block_len: 4096,
            data_len: partition_end,
            external_ivfc_level4: false,
        };

        let meta_diff_len = Diff::calculate_size(&diff_param);

        if let Some(capacity) = quota {
            let meta_block = (divide_up(meta_diff_len, 0x1000)) as u32;
            if meta_block > capacity - 2 {
                return make_error(Error::NoSpace);
            }

            let quota_param = DifiPartitionParam {
                dpfs_level2_block_len: 128,
                dpfs_level3_block_len: 4096,
                ivfc_level1_block_len: 512,
                ivfc_level2_block_len: 512,
                ivfc_level3_block_len: 4096,
                ivfc_level4_block_len: 4096,
                data_len: Quota::BYTE_LEN,
                external_ivfc_level4: true,
            };
            let len = Diff::calculate_size(&quota_param);
            let mut quota_path = ext_path.clone();
            quota_path.push("Quota.dat");
            sd_nand.create(&quota_path, len)?;
            let quota_raw = sd_nand.open(&quota_path, true)?;
            let signer = Box::new(ExtSigner { id, sub_id: None });
            Diff::format(
                quota_raw.clone(),
                Some((signer.clone(), key)),
                &quota_param,
                0x01234567_89ABCDEF,
            )?;

            let quota_file = Diff::new(sd_nand.open(&quota_path, true)?, Some((signer, key)))?;
            write_struct(
                quota_file.partition().as_ref(),
                0,
                Quota {
                    magic: *b"QUOT",
                    version: 0x30000,
                    block_len: 0x1000,
                    dir_capacity: 126,
                    p0: 0,
                    max_block: capacity,
                    p1: 0,
                    // The -2 might come from directory block in FAT16
                    free_block: capacity - meta_block - 2,
                    p2: 0,
                    p3: 0,
                    potential_free_block: capacity - 2,
                    p4: 0,
                    mount_id: 1,
                    p5: 0,
                    p6: 0,
                    p7: 0,
                    mount_len: meta_diff_len as u64,
                },
            )?;
            quota_file.commit()?;
        }

        sd_nand.create(&meta_path, meta_diff_len)?;
        let meta_raw = sd_nand.open(&meta_path, true)?;
        let signer = Box::new(ExtSigner {
            id,
            sub_id: Some(1),
        });
        Diff::format(
            meta_raw.clone(),
            Some((signer.clone(), key)),
            &diff_param,
            0x01234567_89ABCDEF,
        )?;
        let meta_file = Diff::new(meta_raw, Some((signer, key)))?;

        let dir_hash = Rc::new(SubFile::new(
            meta_file.partition().clone(),
            dir_hash_offset,
            param.dir_buckets * 4,
        )?);

        let file_hash = Rc::new(SubFile::new(
            meta_file.partition().clone(),
            file_hash_offset,
            param.file_buckets * 4,
        )?);

        let fat_table = Rc::new(SubFile::new(
            meta_file.partition().clone(),
            fat_offset,
            (data_block_count + 1) * 8,
        )?);

        Fat::format(fat_table.as_ref())?;

        let data = Rc::new(SubFile::new(
            meta_file.partition().clone(),
            data_offset,
            data_block_count * block_len,
        )?);

        let fat = Fat::new(fat_table, data, block_len)?;
        let (dir_table, dir_table_block_index) =
            FatFile::create(fat.clone(), divide_up(dir_table_len, block_len))?;
        let (file_table, file_table_block_index) =
            FatFile::create(fat.clone(), divide_up(file_table_len, block_len))?;
        let dir_table_combo = OffsetOrFatFile {
            block_index: dir_table_block_index as u32,
            block_count: (dir_table.len() / block_len) as u32,
        };
        let file_table_combo = OffsetOrFatFile {
            block_index: file_table_block_index as u32,
            block_count: (file_table.len() / block_len) as u32,
        };
        FsMeta::format(
            dir_hash,
            Rc::new(dir_table),
            param.max_dir + 2,
            file_hash,
            Rc::new(file_table),
            param.max_file + 1,
        )?;

        let header = ExtHeader {
            magic: *b"VSXE",
            version: 0x30000,
            fs_info_offset: ExtHeader::BYTE_LEN as u64,
            image_size: (meta_file.partition().len() / block_len) as u64,
            image_block_len: block_len as u32,
            padding: 0,

            unknown: 0,
            action: 0,
            unknown2: 0,
            mount_id: 0,
            unknown3: 0,
            mount_path: [[0; 0x10]; 0x10],
        };

        write_struct(meta_file.partition().as_ref(), 0, header)?;

        let fs_info = FsInfo {
            unknown: 0,
            block_len: block_len as u32,
            dir_hash_offset: dir_hash_offset as u64,
            dir_buckets: param.dir_buckets as u32,
            p0: 0,
            file_hash_offset: file_hash_offset as u64,
            file_buckets: param.file_buckets as u32,
            p1: 0,
            fat_offset: fat_offset as u64,
            fat_size: data_block_count as u32,
            p2: 0,
            data_offset: data_offset as u64,
            data_block_count: data_block_count as u32,
            p3: 0,
            dir_table: dir_table_combo,
            max_dir: param.max_dir as u32,
            p4: 0,
            file_table: file_table_combo,
            max_file: param.max_file as u32,
            p5: 0,
        };

        write_struct(meta_file.partition().as_ref(), ExtHeader::BYTE_LEN, fs_info)?;
        meta_file.commit()?;

        Ok(())
    }

    pub fn new(
        sd_nand: Rc<SdNandFileSystem>,
        base_path: &[&str],
        id: u64,
        key: [u8; 16],
        has_quota: bool,
        write: bool,
    ) -> Result<ExtData, Error> {
        let id_high = format!("{:08x}", id >> 32);
        let id_low = format!("{:08x}", id & 0xFFFF_FFFF);
        let ext_path: Vec<&str> = base_path
            .iter()
            .cloned()
            .chain([id_high.as_str(), id_low.as_str()].iter().cloned())
            .collect();

        let quota_file = if has_quota {
            let mut quota_path = ext_path.clone();
            quota_path.push("Quota.dat");
            Some(Diff::new(
                sd_nand.open(&quota_path, write)?,
                Some((Box::new(ExtSigner { id, sub_id: None }), key)),
            )?)
        } else {
            None
        };

        let mut meta_path = ext_path;
        meta_path.push("00000000");
        meta_path.push("00000001");

        let meta_file = Diff::new(
            sd_nand.open(&meta_path, write)?,
            Some((
                Box::new(ExtSigner {
                    id,
                    sub_id: Some(1),
                }),
                key,
            )),
        )?;

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
            fs_info.dir_table.block_index as usize,
        )?);

        let file_table: Rc<RandomAccessFile> = Rc::new(FatFile::open(
            fat.clone(),
            fs_info.file_table.block_index as usize,
        )?);

        let fs = FsMeta::new(dir_hash, dir_table, file_hash, file_table)?;

        Ok(ExtData {
            center: Rc::new(ExtDataInner {
                sd_nand,
                base_path: base_path.iter().map(|s| s.to_string()).collect(),
                id,
                fs,
                meta_file,
                quota_file,
                key,
                write,
            }),
        })
    }
}

pub struct File {
    center: Rc<ExtDataInner>,
    meta: FileMeta,
    data: Option<Diff>,
}

impl File {
    fn from_meta(
        center: Rc<ExtDataInner>,
        meta: FileMeta,
        new: Option<(usize, u64)>,
    ) -> Result<File, Error> {
        let file_index = meta.get_ino() + 1;
        let id_high = format!("{:08x}", center.id >> 32);
        let id_low = format!("{:08x}", center.id & 0xFFFF_FFFF);
        let fid_high = file_index / 126;
        let fid_low = file_index % 126;
        let fid_high_s = format!("{:08x}", fid_high);
        let fid_low_s = format!("{:08x}", fid_low);
        let path: Vec<&str> = center
            .base_path
            .iter()
            .map(|s| s as &str)
            .chain(
                [&id_high, &id_low, &fid_high_s, &fid_low_s]
                    .iter()
                    .map(|s| s as &str),
            )
            .collect();

        let mut param = None;
        if let Some((len, _)) = new {
            if len != 0 {
                param = Some(DifiPartitionParam {
                    dpfs_level2_block_len: 128,
                    dpfs_level3_block_len: 4096,
                    ivfc_level1_block_len: 512,
                    ivfc_level2_block_len: 512,
                    ivfc_level3_block_len: 4096,
                    ivfc_level4_block_len: 4096,
                    data_len: len,
                    external_ivfc_level4: true,
                });

                let physical_len = Diff::calculate_size(param.as_ref().unwrap());

                if let Some(quota_file) = center.quota_file.as_ref() {
                    let mut quota: Quota = read_struct(quota_file.partition().as_ref(), 0)?;
                    let block = (divide_up(physical_len, 0x1000)) as u32;
                    if quota.free_block < block {
                        return make_error(Error::NoSpace);
                    }
                    quota.mount_id = file_index as u32;
                    quota.mount_len = physical_len as u64;
                    quota.potential_free_block = quota.free_block;
                    quota.free_block -= block;
                    write_struct(quota_file.partition().as_ref(), 0, quota)?;
                    quota_file.commit()?;
                }

                center.sd_nand.create(&path, physical_len)?
            }
        }
        let file = center.sd_nand.open(&path, center.write).ok();
        let signer = Box::new(ExtSigner {
            id: center.id,
            sub_id: Some((u64::from(fid_high) << 32) | u64::from(fid_low)),
        });

        if let Some((_, unique_id)) = new {
            if let Some(file) = file.as_ref() {
                Diff::format(
                    file.clone(),
                    Some((signer.clone(), center.key)),
                    param.as_ref().unwrap(),
                    unique_id,
                )?;
            }
        }

        let data = file
            .map(|file| Diff::new(file, Some((signer, center.key))))
            .transpose()?;

        let info = meta.get_info()?;
        if data.is_some() && info.unique_id != data.as_ref().unwrap().unique_id() {
            return make_error(Error::UniqueIdMismatch);
        }
        Ok(File { center, meta, data })
    }
}

impl FileSystemFile for File {
    type NameType = [u8; 16];
    type DirType = Dir;

    fn rename(&mut self, parent: &Self::DirType, name: [u8; 16]) -> Result<(), Error> {
        if parent.meta.open_sub_file(name).is_ok() || parent.meta.open_sub_dir(name).is_ok() {
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

    fn resize(&mut self, _len: usize) -> Result<(), Error> {
        make_error(Error::Unsupported)
    }

    fn delete(self) -> Result<(), Error> {
        let file_index = self.meta.get_ino() + 1;
        let physical_len = self.data.as_ref().map_or(0, Diff::parent_len);
        let id_high = format!("{:08x}", self.center.id >> 32);
        let id_low = format!("{:08x}", self.center.id & 0xFFFF_FFFF);
        let fid_high = file_index / 126;
        let fid_low = file_index % 126;
        let fid_high_s = format!("{:08x}", fid_high);
        let fid_low_s = format!("{:08x}", fid_low);
        let path: Vec<&str> = self
            .center
            .base_path
            .iter()
            .map(|s| s as &str)
            .chain(
                [&id_high, &id_low, &fid_high_s, &fid_low_s]
                    .iter()
                    .map(|s| s as &str),
            )
            .collect();

        std::mem::drop(self.data); // close the file first
        self.center.sd_nand.remove(&path)?;
        self.meta.delete()?;

        if let Some(quota_file) = self.center.quota_file.as_ref() {
            let mut quota: Quota = read_struct(quota_file.partition().as_ref(), 0)?;
            quota.mount_id = file_index as u32;
            quota.mount_len = physical_len as u64;
            let block = (divide_up(physical_len, 0x1000)) as u32;
            quota.free_block += block;
            quota.potential_free_block = quota.free_block;
            write_struct(quota_file.partition().as_ref(), 0, quota)?;
            quota_file.commit()?;
        }

        Ok(())
    }

    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error> {
        if pos + buf.len() > self.len() {
            return make_error(Error::OutOfBound);
        }
        if buf.is_empty() {
            return Ok(());
        }
        self.data.as_ref().unwrap().partition().read(pos, buf)
    }

    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error> {
        if pos + buf.len() > self.len() {
            return make_error(Error::OutOfBound);
        }
        if buf.is_empty() {
            return Ok(());
        }
        self.meta.check_exclusive()?;
        self.data.as_ref().unwrap().partition().write(pos, buf)
    }

    fn len(&self) -> usize {
        self.data.as_ref().map_or(0, |f| f.partition().len())
    }

    fn commit(&self) -> Result<(), Error> {
        self.meta.check_exclusive()?;
        if let Some(f) = self.data.as_ref() {
            f.commit()?;
        }
        Ok(())
    }
}

pub struct Dir {
    center: Rc<ExtDataInner>,
    meta: DirMeta,
}

impl FileSystemDir for Dir {
    type NameType = [u8; 16];
    type FileType = File;

    fn rename(&mut self, parent: &Dir, name: [u8; 16]) -> Result<(), Error> {
        if parent.meta.open_sub_file(name).is_ok() || parent.meta.open_sub_dir(name).is_ok() {
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

    fn open_sub_dir(&self, name: [u8; 16]) -> Result<Self, Error> {
        Ok(Dir {
            center: self.center.clone(),
            meta: self.meta.open_sub_dir(name)?,
        })
    }

    fn open_sub_file(&self, name: [u8; 16]) -> Result<Self::FileType, Error> {
        File::from_meta(self.center.clone(), self.meta.open_sub_file(name)?, None)
    }

    fn list_sub_dir(&self) -> Result<Vec<([u8; 16], u32)>, Error> {
        self.meta.list_sub_dir()
    }

    fn list_sub_file(&self) -> Result<Vec<([u8; 16], u32)>, Error> {
        self.meta.list_sub_file()
    }

    fn new_sub_dir(&self, name: [u8; 16]) -> Result<Self, Error> {
        if self.meta.open_sub_file(name).is_ok() || self.meta.open_sub_dir(name).is_ok() {
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

    fn new_sub_file(&self, name: [u8; 16], len: usize) -> Result<Self::FileType, Error> {
        if self.meta.open_sub_file(name).is_ok() || self.meta.open_sub_dir(name).is_ok() {
            return make_error(Error::AlreadyExist);
        }
        let unique_id = 0xDEAD_BEEF;
        let meta = self.meta.new_sub_file(
            name,
            ExtFile {
                next: 0,
                padding1: 0,
                block: 0x8000_0000,
                unique_id,
                padding2: 0,
            },
        )?;
        File::from_meta(self.center.clone(), meta, Some((len, unique_id)))
    }

    fn delete(self) -> Result<(), Error> {
        self.meta.delete()
    }
}

impl FileSystem for ExtData {
    type FileType = File;
    type DirType = Dir;
    type NameType = [u8; 16];

    fn open_file(&self, ino: u32) -> Result<Self::FileType, Error> {
        let meta = FileMeta::open_ino(self.center.fs.clone(), ino)?;
        File::from_meta(self.center.clone(), meta, None)
    }

    fn open_dir(&self, ino: u32) -> Result<Self::DirType, Error> {
        let meta = DirMeta::open_ino(self.center.fs.clone(), ino)?;
        Ok(Dir {
            center: self.center.clone(),
            meta,
        })
    }

    fn commit(&self) -> Result<(), Error> {
        self.center.meta_file.commit()
    }
}

#[cfg(test)]
mod test {
    use crate::ext_data::*;
    #[test]
    fn struct_size() {
        assert_eq!(ExtHeader::BYTE_LEN, 0x138);
        assert_eq!(ExtFile::BYTE_LEN, 24);
        assert_eq!(Quota::BYTE_LEN, 0x48);
    }

    fn gen_name() -> [u8; 16] {
        use rand::prelude::*;
        let mut rng = rand::thread_rng();
        let mut name = [0; 16];
        name[0] = rng.gen_range(0, 5);
        name
    }

    fn gen_len() -> usize {
        use rand::prelude::*;
        let mut rng = rand::thread_rng();
        if rng.gen_range(0, 5) == 0 {
            0
        } else {
            rng.gen_range(0, 4096 * 5)
        }
    }

    #[test]
    fn fs_fuzz() {
        use rand::prelude::*;
        let mut rng = rand::thread_rng();

        for _ in 0..10 {
            let nand = Rc::new(crate::sd_nand_common::test::VirtualFileSystem::new());

            let param = ExtDataFormatParam {
                max_dir: rng.gen_range(10, 100),
                dir_buckets: rng.gen_range(10, 100),
                max_file: rng.gen_range(10, 100),
                file_buckets: rng.gen_range(10, 100),
            };

            ExtData::format(nand.as_ref(), &[], 0, [0; 16], None, &param).unwrap();
            let file_system = ExtData::new(nand.clone(), &[], 0, [0; 16], false, true).unwrap();
            crate::file_system::test::fuzzer(
                file_system,
                param.max_dir as usize,
                param.max_file as usize,
                || ExtData::new(nand.clone(), &[], 0, [0; 16], false, true).unwrap(),
                gen_name,
                gen_len,
            );
        }
    }
}
