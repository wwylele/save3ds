use crate::diff::Diff;
use crate::error::*;
use crate::fat::*;
use crate::fs_meta::{self, FileInfo};
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

pub struct ExtData {
    sd_nand: Rc<SdNandFileSystem>,
    base_path: Vec<String>,
    id: u64,
    fs: Rc<FsMeta>,
    meta_file: Diff,
    key: [u8; 16],
}

impl ExtData {
    pub fn new(
        sd_nand: Rc<SdNandFileSystem>,
        base_path: Vec<String>,
        id: u64,
        key: [u8; 16],
    ) -> Result<Rc<ExtData>, Error> {
        let id_high = format!("{:08x}", id >> 32);
        let id_low = format!("{:08x}", id & 0xFFFF_FFFF);
        let meta_path: Vec<&str> = base_path
            .iter()
            .map(|s| s as &str)
            .chain([&id_high, &id_low, "00000000", "00000001"].iter().cloned())
            .collect();
        let meta_file = Diff::new(
            sd_nand.open(&meta_path)?,
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
            (fs_info.dir_table & 0xFFFF_FFFF) as usize,
        )?);

        let file_table: Rc<RandomAccessFile> = Rc::new(FatFile::open(
            fat.clone(),
            (fs_info.file_table & 0xFFFF_FFFF) as usize,
        )?);

        let fs = FsMeta::new(dir_hash, dir_table, file_hash, file_table)?;

        Ok(Rc::new(ExtData {
            sd_nand,
            base_path,
            id,
            fs,
            meta_file,
            key,
        }))
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
        let data = Diff::new(
            center.sd_nand.open(&path)?,
            Some((
                Box::new(ExtSigner {
                    id: center.id,
                    sub_id: Some((u64::from(fid_high) << 32) | u64::from(fid_low)),
                }),
                center.key,
            )),
        )?;

        let info = meta.get_info()?;
        if info.unique_id != data.unique_id() {
            return make_error(Error::UniqueIdMismatch);
        }
        Ok(File { center, meta, data })
    }

    pub fn open_ino(center: Rc<ExtData>, ino: u32) -> Result<File, Error> {
        let meta = FileMeta::open_ino(center.fs.clone(), ino)?;
        File::from_meta(center, meta)
    }
}

pub struct Dir {
    center: Rc<ExtData>,
    meta: DirMeta,
}

pub struct ExtDataFileSystem {}
impl FileSystem for ExtDataFileSystem {
    type CenterType = ExtData;
    type FileType = File;
    type DirType = Dir;

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

    fn read(file: &Self::FileType, pos: usize, buf: &mut [u8]) -> Result<(), Error> {
        file.data.partition().read(pos, buf)
    }

    fn write(file: &Self::FileType, pos: usize, buf: &[u8]) -> Result<(), Error> {
        file.data.partition().write(pos, buf)
    }

    fn len(file: &Self::FileType) -> usize {
        file.data.partition().len()
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

    fn dir_delete(dir: Self::DirType) -> Result<(), Error> {
        dir.meta.delete()
    }

    fn commit(center: &Self::CenterType) -> Result<(), Error> {
        center.meta_file.commit()
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
