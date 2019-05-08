use crate::disa::Disa;
use crate::fat::*;
use crate::fs;
use crate::random_access_file::*;
use crate::sub_file::SubFile;
use byte_struct::*;
use std::rc::Rc;

type FsMeta = fs::FsMeta<fs::SaveKey, fs::SaveDir, fs::SaveKey, fs::SaveFile>;
type DirMeta = fs::DirMeta<fs::SaveKey, fs::SaveDir, fs::SaveKey, fs::SaveFile>;
type FileMeta = fs::FileMeta<fs::SaveKey, fs::SaveDir, fs::SaveKey, fs::SaveFile>;

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

#[derive(ByteStruct)]
#[byte_struct_le]
struct FsInfo {
    unknown: u32,
    block_len: u32,
    dir_hash_offset: u64,
    dir_buckets: u32,
    p0: u32,
    file_hash_offset: u64,
    file_buckets: u32,
    p1: u32,
    fat_offset: u64,
    fat_size: u32,
    p2: u32,
    data_offset: u64,
    data_block_count: u32,
    p3: u32,
    dir_table: u64,
    max_dir: u32,
    p4: u32,
    file_table: u64,
    max_file: u32,
    p5: u32,
}

pub struct SaveData {
    disa: Rc<Disa>,
    fat: Rc<Fat>,
    fs: Rc<FsMeta>,
    block_len: usize,
}

impl SaveData {
    pub fn new(file: Rc<RandomAccessFile>) -> Result<Rc<SaveData>, Error> {
        let disa = Rc::new(Disa::new(file)?);
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
                (fs_info.max_dir + 2) as usize
                    * (fs::SaveKey::BYTE_LEN + fs::SaveDir::BYTE_LEN + 4),
            )?)
        } else {
            let block = (fs_info.dir_table & 0xFFFF_FFFF) as usize;
            Rc::new(FatFile::open(fat.clone(), block)?)
        };

        let file_table: Rc<RandomAccessFile> = if disa.partition_count() == 2 {
            Rc::new(SubFile::new(
                disa[0].clone(),
                fs_info.file_table as usize,
                (fs_info.max_file + 1) as usize
                    * (fs::SaveKey::BYTE_LEN + fs::SaveFile::BYTE_LEN + 4),
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

        let mut info = self.meta.get_info()?;

        if self.len == 0 {
            // zero => non-zero
            let (fat_file, block) = FatFile::create(
                self.center.fat.clone(),
                1 + (len - 1) / self.center.block_len,
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
                .resize(1 + (len - 1) / self.center.block_len)?;
        }

        info.size = len as u64;
        self.meta.set_info(info)?;

        self.len = len;

        Ok(())
    }
}

impl RandomAccessFile for File {
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
    center: Rc<SaveData>,
    meta: DirMeta,
}

impl Dir {
    pub fn open_root(center: Rc<SaveData>) -> Result<Dir, Error> {
        let meta = DirMeta::open_root(center.fs.clone())?;
        Ok(Dir { center, meta })
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

    pub fn list_sub_dir(&self) -> Result<Vec<[u8; 16]>, Error> {
        self.meta.list_sub_dir()
    }

    pub fn list_sub_file(&self) -> Result<Vec<[u8; 16]>, Error> {
        self.meta.list_sub_file()
    }

    pub fn new_sub_dir(&self, name: [u8; 16]) -> Result<Dir, Error> {
        let dir_info = fs::SaveDir {
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

    pub fn new_sub_file(&self, name: [u8; 16], len: usize) -> Result<File, Error> {
        let (fat_file, block) = if len == 0 {
            (None, 0x8000_0000)
        } else {
            let (fat_file, block) = FatFile::create(
                self.center.fat.clone(),
                1 + (len - 1) / self.center.block_len,
            )?;
            (Some(fat_file), block as u32)
        };
        match self.meta.new_sub_file(
            name,
            fs::SaveFile {
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
            Ok(meta) => File::from_meta(self.center.clone(), meta),
        }
    }

    pub fn delete(self) -> Result<Option<Dir>, Error> {
        if let Some(meta) = self.meta.delete()? {
            Ok(Some(Dir {
                center: self.center,
                meta,
            }))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod test {
    use crate::save_data::*;
    #[test]
    fn struct_size() {
        assert_eq!(SaveHeader::BYTE_LEN, 0x20);
        assert_eq!(FsInfo::BYTE_LEN, 0x68);
    }

}