use crate::byte_struct_common::*;
use crate::error::*;
use crate::random_access_file::*;
use byte_struct::*;
use std::cell::*;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;

#[derive(ByteStruct)]
#[byte_struct_le]
pub struct OffsetOrFatFile {
    pub block_index: u32,
    pub block_count: u32,
}

impl OffsetOrFatFile {
    pub fn from_offset(offset: usize) -> OffsetOrFatFile {
        OffsetOrFatFile {
            block_index: (offset & 0xFFFF_FFFF) as u32,
            block_count: (offset >> 32) as u32,
        }
    }

    pub fn to_offset(&self) -> usize {
        self.block_index as usize | ((self.block_count as usize) << 32)
    }
}

#[derive(ByteStruct)]
#[byte_struct_le]
pub struct FsInfo {
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
    pub dir_table: OffsetOrFatFile,
    pub max_dir: u32,
    pub p4: u32,
    pub file_table: OffsetOrFatFile,
    pub max_file: u32,
    pub p5: u32,
}

struct RefTicket<KeyType, InfoType> {
    index: u32,
    ref_count: Rc<RefCell<HashMap<u32, u32>>>,

    phantom_key: PhantomData<KeyType>,
    phantom_info: PhantomData<InfoType>,
}

impl<KeyType, InfoType> Drop for RefTicket<KeyType, InfoType> {
    fn drop(&mut self) {
        let mut ref_count = self.ref_count.borrow_mut();
        let previous = *ref_count.get(&self.index).unwrap();
        if previous == 1 {
            ref_count.remove(&self.index);
        } else {
            ref_count.insert(self.index, previous - 1);
        }
    }
}

impl<KeyType, InfoType> RefTicket<KeyType, InfoType> {
    pub fn check_exclusive(&self) -> Result<(), Error> {
        if *self.ref_count.borrow().get(&self.index).unwrap() != 1 {
            make_error(Error::Busy)
        } else {
            Ok(())
        }
    }
}

pub struct MetaTableStat {
    pub total: usize,
    pub free: usize,
}

struct MetaTable<KeyType, InfoType> {
    hash: Rc<dyn RandomAccessFile>,
    table: Rc<dyn RandomAccessFile>,

    buckets: usize,

    entry_len: usize,
    eo_info: usize,
    eo_collision: usize,

    ref_count: Rc<RefCell<HashMap<u32, u32>>>,

    phantom_key: PhantomData<KeyType>,
    phantom_info: PhantomData<InfoType>,
}

impl<KeyType: ByteStruct + PartialEq, InfoType: ByteStruct> MetaTable<KeyType, InfoType> {
    fn format(
        hash: &dyn RandomAccessFile,
        table: &dyn RandomAccessFile,
        entry_count: usize,
    ) -> Result<(), Error> {
        hash.write(0, &vec![0; hash.len()])?;

        write_struct(table, 0, U32le { v: 1 })?;
        write_struct(
            table,
            4,
            U32le {
                v: entry_count as u32,
            },
        )?;

        let padding = KeyType::BYTE_LEN + InfoType::BYTE_LEN - 8;
        if padding > 0 {
            table.write(8, &vec![0; padding])?;
        }

        write_struct(
            table,
            KeyType::BYTE_LEN + InfoType::BYTE_LEN,
            U32le { v: 0 },
        )
    }

    fn new(
        hash: Rc<dyn RandomAccessFile>,
        table: Rc<dyn RandomAccessFile>,
    ) -> Result<MetaTable<KeyType, InfoType>, Error> {
        assert!(KeyType::BYTE_LEN % 4 == 0);

        if hash.len() % 4 != 0 {
            return make_error(Error::SizeMismatch);
        }

        let buckets = hash.len() / 4;

        let entry_len = KeyType::BYTE_LEN + InfoType::BYTE_LEN + 4;
        let eo_info = KeyType::BYTE_LEN;
        let eo_collision = KeyType::BYTE_LEN + InfoType::BYTE_LEN;

        Ok(MetaTable {
            hash,
            table,
            buckets,
            entry_len,
            eo_info,
            eo_collision,
            ref_count: Rc::new(RefCell::new(HashMap::new())),
            phantom_key: PhantomData,
            phantom_info: PhantomData,
        })
    }

    fn hash(&self, key: &KeyType) -> usize {
        let mut h = 0x1234_5678;
        let mut bytes = vec![0; KeyType::BYTE_LEN];
        key.write_bytes(&mut bytes);
        for i in (0..KeyType::BYTE_LEN).step_by(4) {
            h = (h >> 1) | (h << 31);
            h ^= u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
        }
        h as usize % self.buckets
    }

    fn get(&self, key: &KeyType) -> Result<(InfoType, u32), Error> {
        let h = self.hash(key);
        let table = self.table.as_ref();
        let hash = self.hash.as_ref();
        let mut index = read_struct::<U32le>(hash, h * 4)?.v;
        while index != 0 {
            let entry_offset = index as usize * self.entry_len;
            let other_key: KeyType = read_struct(table, entry_offset)?;
            if *key == other_key {
                let info = read_struct(table, entry_offset + self.eo_info)?;
                return Ok((info, index));
            }

            index = read_struct::<U32le>(table, entry_offset + self.eo_collision)?.v;
        }
        make_error(Error::NotFound)
    }

    fn get_at(&self, index: u32) -> Result<(InfoType, KeyType), Error> {
        let entry_offset = index as usize * self.entry_len;
        let table = self.table.as_ref();
        let info = read_struct(table, entry_offset + self.eo_info)?;
        let key = read_struct(table, entry_offset)?;
        Ok((info, key))
    }

    fn set(&self, index: u32, info: InfoType) -> Result<(), Error> {
        let entry_offset = index as usize * self.entry_len;
        write_struct(self.table.as_ref(), entry_offset + self.eo_info, info)
    }

    fn remove(&self, index: u32) -> Result<(), Error> {
        let entry_offset = index as usize * self.entry_len;
        let table = self.table.as_ref();
        let hash = self.hash.as_ref();
        let key: KeyType = read_struct(table, entry_offset)?;
        let collision = read_struct::<U32le>(table, entry_offset + self.eo_collision)?.v;

        // scan the collision list and relink it
        let h = self.hash(&key);
        let mut prev = (hash, h * 4);
        loop {
            let other = read_struct::<U32le>(prev.0, prev.1)?.v;
            assert!(other != 0);
            if other == index {
                write_struct(prev.0, prev.1, U32le { v: collision })?;
                break;
            }
            prev = (table, other as usize * self.entry_len + self.eo_collision);
        }

        // make a dummy entry and link it
        let mut dummy = vec![0; self.entry_len];
        table.read(0, &mut dummy)?;
        table.write(entry_offset, &dummy)?;
        write_struct(table, self.eo_collision, U32le { v: index })?;

        Ok(())
    }

    fn add(&self, key: KeyType, info: InfoType) -> Result<u32, Error> {
        match self.get(&key) {
            Err(Error::NotFound) => {}
            Ok(_) => return make_error(Error::AlreadyExist),
            Err(e) => return Err(e),
        }
        let table = self.table.as_ref();
        let hash = self.hash.as_ref();
        let mut index = read_struct::<U32le>(table, self.eo_collision)?.v;
        let entry_offset = if index == 0 {
            let entry_count = read_struct::<U32le>(table, 0)?.v;
            let max_entry_count = read_struct::<U32le>(table, 4)?.v;
            if entry_count == max_entry_count {
                return make_error(Error::NoSpace);
            }
            write_struct(table, 0, U32le { v: entry_count + 1 })?;
            index = entry_count;
            index as usize * self.entry_len
        } else {
            let entry_offset = index as usize * self.entry_len;
            let next_dummy = read_struct::<U32le>(table, entry_offset + self.eo_collision)?;
            write_struct(table, self.eo_collision, next_dummy)?;
            entry_offset
        };

        let h = self.hash(&key);
        let collistion = read_struct::<U32le>(hash, h * 4)?;
        write_struct(hash, h * 4, U32le { v: index })?;
        write_struct(table, entry_offset, key)?;
        write_struct(table, entry_offset + self.eo_info, info)?;
        write_struct(table, entry_offset + self.eo_collision, collistion)?;

        Ok(index)
    }

    fn stat(&self) -> Result<MetaTableStat, Error> {
        let table = self.table.as_ref();
        let entry_count = read_struct::<U32le>(table, 0)?.v;
        let max_entry_count = read_struct::<U32le>(table, 4)?.v;
        let mut index = read_struct::<U32le>(table, self.eo_collision)?.v;
        let mut dummy_count = 0;
        while index != 0 {
            dummy_count += 1;
            let entry_offset = index as usize * self.entry_len;
            index = read_struct::<U32le>(table, entry_offset + self.eo_collision)?.v;
        }

        Ok(MetaTableStat {
            total: max_entry_count as usize - 1,
            free: (max_entry_count - entry_count + dummy_count) as usize,
        })
    }

    pub fn acquire_ticket(&self, index: u32) -> RefTicket<KeyType, InfoType> {
        let mut ref_count = self.ref_count.borrow_mut();
        let previous = ref_count.get(&index).cloned().unwrap_or(0);
        ref_count.insert(index, previous + 1);
        RefTicket {
            index,
            ref_count: self.ref_count.clone(),
            phantom_key: PhantomData,
            phantom_info: PhantomData,
        }
    }
}

pub trait ParentedKey: ByteStruct + PartialEq + Clone {
    type NameType: PartialEq + Default;
    fn get_parent(&self) -> u32;
    fn get_name(&self) -> Self::NameType;
    fn new(parent: u32, name: Self::NameType) -> Self;
    fn new_root() -> Self {
        Self::new(0, Self::NameType::default())
    }
}

pub trait FileInfo: ByteStruct + Clone {
    fn set_next(&mut self, index: u32);
    fn get_next(&self) -> u32;
}

pub trait DirInfo: ByteStruct + Clone {
    fn set_sub_dir(&mut self, index: u32);
    fn get_sub_dir(&self) -> u32;
    fn set_sub_file(&mut self, index: u32);
    fn get_sub_file(&self) -> u32;
    fn set_next(&mut self, index: u32);
    fn get_next(&self) -> u32;
    fn new_root() -> Self;
}

pub struct MetaStat {
    pub dirs: MetaTableStat,
    pub files: MetaTableStat,
}

pub struct FsMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType> {
    dirs: MetaTable<DirKeyType, DirInfoType>,
    files: MetaTable<FileKeyType, FileInfoType>,
}

impl<
        DirKeyType: ParentedKey,
        DirInfoType: DirInfo,
        FileKeyType: ParentedKey,
        FileInfoType: FileInfo,
    > FsMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType>
{
    pub fn format(
        dir_hash: Rc<dyn RandomAccessFile>,
        dir_table: Rc<dyn RandomAccessFile>,
        dir_entry_count: usize,
        file_hash: Rc<dyn RandomAccessFile>,
        file_table: Rc<dyn RandomAccessFile>,
        file_entry_count: usize,
    ) -> Result<(), Error> {
        MetaTable::<DirKeyType, DirInfoType>::format(
            dir_hash.as_ref(),
            dir_table.as_ref(),
            dir_entry_count,
        )?;
        MetaTable::<FileKeyType, FileInfoType>::format(
            file_hash.as_ref(),
            file_table.as_ref(),
            file_entry_count,
        )?;
        let dirs = MetaTable::new(dir_hash, dir_table)?;
        dirs.add(DirKeyType::new_root(), DirInfoType::new_root())?;
        Ok(())
    }

    pub fn new(
        dir_hash: Rc<dyn RandomAccessFile>,
        dir_table: Rc<dyn RandomAccessFile>,
        file_hash: Rc<dyn RandomAccessFile>,
        file_table: Rc<dyn RandomAccessFile>,
    ) -> Result<Rc<FsMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType>>, Error> {
        Ok(Rc::new(FsMeta {
            dirs: MetaTable::new(dir_hash, dir_table)?,
            files: MetaTable::new(file_hash, file_table)?,
        }))
    }

    pub fn stat(&self) -> Result<MetaStat, Error> {
        Ok(MetaStat {
            dirs: self.dirs.stat()?,
            files: self.files.stat()?,
        })
    }
}

pub struct FileMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType> {
    ticket: RefTicket<FileKeyType, FileInfoType>,
    fs: Rc<FsMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType>>,
}

impl<
        DirKeyType: ParentedKey,
        DirInfoType: DirInfo,
        FileKeyType: ParentedKey,
        FileInfoType: FileInfo,
    > FileMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType>
{
    pub fn open_ino(
        fs: Rc<FsMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType>>,
        ino: u32,
    ) -> Result<Self, Error> {
        let ticket = fs.files.acquire_ticket(ino);
        Ok(FileMeta { ticket, fs })
    }

    pub fn rename(
        &mut self,
        parent: &DirMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType>,
        name: FileKeyType::NameType,
    ) -> Result<(), Error> {
        let (info, _) = self.fs.files.get_at(self.ticket.index)?;
        // Note: we don't check_exclusive on rename
        // because the consecutive delete-new operation preserves ino
        self.delete_impl()?;
        *self = parent.new_sub_file(name, info)?;
        Ok(())
    }

    pub fn get_parent_ino(&self) -> Result<u32, Error> {
        let (_, key) = self.fs.files.get_at(self.ticket.index)?;
        Ok(key.get_parent())
    }

    pub fn get_ino(&self) -> u32 {
        self.ticket.index
    }

    pub fn get_info(&self) -> Result<FileInfoType, Error> {
        Ok(self.fs.files.get_at(self.ticket.index)?.0)
    }

    pub fn set_info(&self, info: FileInfoType) -> Result<(), Error> {
        self.fs.files.set(self.ticket.index, info)
    }

    pub fn delete(self) -> Result<(), Error> {
        self.ticket.check_exclusive()?;
        self.delete_impl()
    }
    fn delete_impl(&self) -> Result<(), Error> {
        let (self_info, _) = self.fs.files.get_at(self.ticket.index)?;

        let parent_index = self.get_parent_ino()?;
        let (mut parent, _) = self.fs.dirs.get_at(parent_index)?;
        let mut head_index = parent.get_sub_file();
        if head_index == self.ticket.index {
            parent.set_sub_file(self_info.get_next());
            self.fs.dirs.set(parent_index, parent)?;
        } else {
            loop {
                assert!(head_index != 0);
                let (mut head, _) = self.fs.files.get_at(head_index)?;
                let next_index = head.get_next();
                if next_index == self.ticket.index {
                    head.set_next(self_info.get_next());
                    self.fs.files.set(head_index, head)?;
                    break;
                }
                head_index = next_index;
            }
        }

        self.fs.files.remove(self.ticket.index)?;

        Ok(())
    }

    pub fn check_exclusive(&self) -> Result<(), Error> {
        self.ticket.check_exclusive()
    }
}

pub struct DirMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType> {
    ticket: RefTicket<DirKeyType, DirInfoType>,
    fs: Rc<FsMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType>>,
}

impl<
        DirKeyType: ParentedKey,
        DirInfoType: DirInfo,
        FileKeyType: ParentedKey,
        FileInfoType: FileInfo,
    > DirMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType>
{
    pub fn open_ino(
        fs: Rc<FsMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType>>,
        ino: u32,
    ) -> Result<Self, Error> {
        let ticket = fs.dirs.acquire_ticket(ino);
        Ok(DirMeta { ticket, fs })
    }

    pub fn rename(
        &mut self,
        parent: &DirMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType>,
        name: DirKeyType::NameType,
    ) -> Result<(), Error> {
        let (info, _) = self.fs.dirs.get_at(self.ticket.index)?;
        // Note: we don't check_exclusive on rename
        // because the consecutive delete-new operation preserves ino
        self.delete_impl()?;
        *self = parent.new_sub_dir_impl(name, info, false)?;
        Ok(())
    }

    pub fn get_parent_ino(&self) -> Result<u32, Error> {
        let (_, key) = self.fs.dirs.get_at(self.ticket.index)?;
        Ok(key.get_parent())
    }

    pub fn get_ino(&self) -> u32 {
        self.ticket.index
    }

    pub fn open_sub_dir(&self, name: DirKeyType::NameType) -> Result<Self, Error> {
        let key = DirKeyType::new(self.ticket.index, name);
        let (_, pos) = self.fs.dirs.get(&key)?;
        let ticket = self.fs.dirs.acquire_ticket(pos);
        Ok(DirMeta {
            ticket,
            fs: self.fs.clone(),
        })
    }

    pub fn open_sub_file(
        &self,
        name: FileKeyType::NameType,
    ) -> Result<FileMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType>, Error> {
        let key = FileKeyType::new(self.ticket.index, name);
        let (_, pos) = self.fs.files.get(&key)?;
        let ticket = self.fs.files.acquire_ticket(pos);
        Ok(FileMeta {
            ticket,
            fs: self.fs.clone(),
        })
    }

    pub fn list_sub_dir(&self) -> Result<Vec<(DirKeyType::NameType, u32)>, Error> {
        let (self_info, _) = self.fs.dirs.get_at(self.ticket.index)?;
        let mut index = self_info.get_sub_dir();
        let mut result = vec![];
        while index != 0 {
            let (info, key) = self.fs.dirs.get_at(index)?;
            result.push((key.get_name(), index));
            index = info.get_next();
        }
        Ok(result)
    }

    pub fn list_sub_file(&self) -> Result<Vec<(FileKeyType::NameType, u32)>, Error> {
        let (self_info, _) = self.fs.dirs.get_at(self.ticket.index)?;
        let mut index = self_info.get_sub_file();
        let mut result = vec![];
        while index != 0 {
            let (info, key) = self.fs.files.get_at(index)?;
            result.push((key.get_name(), index));
            index = info.get_next();
        }
        Ok(result)
    }

    pub fn new_sub_dir(
        &self,
        name: DirKeyType::NameType,
        info: DirInfoType,
    ) -> Result<Self, Error> {
        self.new_sub_dir_impl(name, info, true)
    }

    fn new_sub_dir_impl(
        &self,
        name: DirKeyType::NameType,
        mut info: DirInfoType,
        reset_sub_info: bool,
    ) -> Result<Self, Error> {
        let (mut self_info, _) = self.fs.dirs.get_at(self.ticket.index)?;
        let key = DirKeyType::new(self.ticket.index, name);
        info.set_next(self_info.get_sub_dir());
        if reset_sub_info {
            info.set_sub_dir(0);
            info.set_sub_file(0);
        }
        let pos = self.fs.dirs.add(key.clone(), info)?;
        self_info.set_sub_dir(pos);
        self.fs.dirs.set(self.ticket.index, self_info.clone())?;
        let ticket = self.fs.dirs.acquire_ticket(pos);
        Ok(DirMeta {
            ticket,
            fs: self.fs.clone(),
        })
    }

    pub fn new_sub_file(
        &self,
        name: FileKeyType::NameType,
        mut info: FileInfoType,
    ) -> Result<FileMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType>, Error> {
        let (mut self_info, _) = self.fs.dirs.get_at(self.ticket.index)?;
        let key = FileKeyType::new(self.ticket.index, name);
        info.set_next(self_info.get_sub_file());
        let pos = self.fs.files.add(key.clone(), info)?;
        self_info.set_sub_file(pos);
        self.fs.dirs.set(self.ticket.index, self_info.clone())?;
        let ticket = self.fs.files.acquire_ticket(pos);
        Ok(FileMeta {
            ticket,
            fs: self.fs.clone(),
        })
    }

    pub fn delete(self) -> Result<(), Error> {
        self.ticket.check_exclusive()?;
        let (self_info, _) = self.fs.dirs.get_at(self.ticket.index)?;
        if self.ticket.index == 1 {
            return make_error(Error::DeletingRoot);
        }
        if self_info.get_sub_dir() != 0 {
            return make_error(Error::NotEmpty);
        }
        if self_info.get_sub_file() != 0 {
            return make_error(Error::NotEmpty);
        }
        self.delete_impl()?;
        Ok(())
    }

    fn delete_impl(&self) -> Result<(), Error> {
        let (self_info, _) = self.fs.dirs.get_at(self.ticket.index)?;
        let parent_index = self.get_parent_ino()?;
        let (mut parent, _) = self.fs.dirs.get_at(parent_index)?;
        let mut head_index = parent.get_sub_dir();
        if head_index == self.ticket.index {
            parent.set_sub_dir(self_info.get_next());
            self.fs.dirs.set(parent_index, parent)?;
        } else {
            loop {
                assert!(head_index != 0);
                let (mut head, _) = self.fs.dirs.get_at(head_index)?;
                let next_index = head.get_next();
                if next_index == self.ticket.index {
                    head.set_next(self_info.get_next());
                    self.fs.dirs.set(head_index, head)?;
                    break;
                }
                head_index = next_index;
            }
        }

        self.fs.dirs.remove(self.ticket.index)?;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::fs_meta::*;
    use crate::memory_file::MemoryFile;
    use rand::prelude::*;
    use std::collections::HashSet;
    use std::iter::*;

    fn borrow_mut_two<T>(a: &mut [T], i: usize, j: usize) -> (&mut T, &mut T) {
        assert!(i != j);
        if i > j {
            let (item_j, item_i) = borrow_mut_two(a, j, i);
            return (item_i, item_j);
        }
        let (left, right) = a.split_at_mut(j);
        (&mut left[i], &mut right[0])
    }

    #[allow(clippy::cognitive_complexity)]
    #[test]
    fn fs_fuzz() {
        use crate::save_data::SaveFile;
        use crate::save_ext_common::*;
        let mut rng = rand::thread_rng();
        for _ in 0..100 {
            let dir_entry_count = rng.gen_range(10, 1000);
            let dir_buckets = rng.gen_range(10, 100);
            let dir_hash = Rc::new(MemoryFile::new(vec![0; dir_buckets * 4]));
            let dir_table = Rc::new(MemoryFile::new(vec![
                0;
                dir_entry_count
                    * (SaveExtDir::BYTE_LEN
                        + SaveExtKey::BYTE_LEN
                        + 4)
            ]));

            let file_entry_count = rng.gen_range(10, 1000);
            let file_buckets = rng.gen_range(10, 100);
            let file_hash = Rc::new(MemoryFile::new(vec![0; file_buckets * 4]));
            let file_table = Rc::new(MemoryFile::new(vec![
                0;
                file_entry_count
                    * (SaveFile::BYTE_LEN
                        + SaveExtKey::BYTE_LEN
                        + 4)
            ]));

            FsMeta::<SaveExtKey, SaveExtDir, SaveExtKey, SaveFile>::format(
                dir_hash.clone(),
                dir_table.clone(),
                dir_entry_count,
                file_hash.clone(),
                file_table.clone(),
                file_entry_count,
            )
            .unwrap();

            let fs = FsMeta::<SaveExtKey, SaveExtDir, SaveExtKey, SaveFile>::new(
                dir_hash, dir_table, file_hash, file_table,
            )
            .unwrap();

            struct Dir {
                meta: DirMeta<SaveExtKey, SaveExtDir, SaveExtKey, SaveFile>,
                name: [u8; 16],
                parent: usize,
                sub_dir_name: HashSet<[u8; 16]>,
                sub_file_name: HashSet<[u8; 16]>,
            }

            struct File {
                meta: FileMeta<SaveExtKey, SaveExtDir, SaveExtKey, SaveFile>,
                name: [u8; 16],
                parent: usize,
            }

            let mut dirs = vec![Dir {
                meta: DirMeta::<SaveExtKey, SaveExtDir, SaveExtKey, SaveFile>::open_ino(
                    fs.clone(),
                    1,
                )
                .unwrap(),
                name: [0; 16],
                parent: 0xFFFF_FFFF,
                sub_dir_name: HashSet::new(),
                sub_file_name: HashSet::new(),
            }];

            let mut files: Vec<File> = vec![];

            for _ in 0..1000 {
                match rng.gen_range(0, 9) {
                    0 => {
                        // open_sub_dir
                        if dirs.len() == 1 {
                            continue;
                        }
                        let index = rng.gen_range(1, dirs.len());
                        dirs[index].meta = dirs[dirs[index].parent]
                            .meta
                            .open_sub_dir(dirs[index].name)
                            .unwrap();
                    }
                    1 => {
                        // new_sub_dir
                        let parent = rng.gen_range(0, dirs.len());
                        let name = loop {
                            let name: [u8; 16] = rng.gen();
                            if !dirs[parent].sub_dir_name.contains(&name) {
                                break name;
                            }
                        };
                        match dirs[parent].meta.new_sub_dir(
                            name,
                            SaveExtDir {
                                next: 0,
                                sub_dir: 0,
                                sub_file: 0,
                                padding: 0,
                            },
                        ) {
                            Err(Error::NoSpace) => assert_eq!(dirs.len(), dir_entry_count - 1),
                            Ok(meta) => {
                                assert!(dirs.len() < dir_entry_count - 1);
                                assert!(dirs[parent].sub_dir_name.insert(name));
                                dirs.push(Dir {
                                    meta,
                                    name,
                                    parent,
                                    sub_dir_name: HashSet::new(),
                                    sub_file_name: HashSet::new(),
                                })
                            }
                            _ => unreachable!(),
                        }
                    }
                    2 => {
                        // delete dir
                        if dirs.len() == 1 {
                            continue;
                        }
                        let index = rng.gen_range(1, dirs.len());
                        let mut dir = dirs.remove(index);
                        let ino = dir.meta.get_ino();
                        match dir.meta.delete() {
                            Ok(()) => {
                                assert!(
                                    dir.sub_dir_name.is_empty() && dir.sub_file_name.is_empty()
                                );
                                let mut parent = dir.parent;
                                if parent > index {
                                    parent -= 1;
                                }
                                assert!(dirs[parent].sub_dir_name.remove(&dir.name));
                                for dir in dirs.iter_mut() {
                                    if dir.parent > index && dir.parent != 0xFFFF_FFFF {
                                        dir.parent -= 1;
                                    }
                                }

                                for file in files.iter_mut() {
                                    if file.parent > index {
                                        file.parent -= 1;
                                    }
                                }
                            }
                            Err(Error::NotEmpty) => {
                                assert!(
                                    !dir.sub_dir_name.is_empty() || !dir.sub_file_name.is_empty()
                                );
                                dir.meta = DirMeta::open_ino(fs.clone(), ino).unwrap();
                                dirs.insert(index, dir);
                            }
                            _ => unreachable!(),
                        }
                    }
                    3 => {
                        // list_sub_dir
                        let index = rng.gen_range(0, dirs.len());
                        assert_eq!(
                            HashSet::from_iter(
                                dirs[index]
                                    .meta
                                    .list_sub_dir()
                                    .unwrap()
                                    .into_iter()
                                    .map(|n| n.0)
                            ),
                            dirs[index].sub_dir_name
                        );
                        assert_eq!(
                            HashSet::from_iter(
                                dirs[index]
                                    .meta
                                    .list_sub_file()
                                    .unwrap()
                                    .into_iter()
                                    .map(|n| n.0)
                            ),
                            dirs[index].sub_file_name
                        );
                    }
                    4 => {
                        // open_sub_file
                        if files.is_empty() {
                            continue;
                        }
                        let index = rng.gen_range(0, files.len());
                        files[index].meta = dirs[files[index].parent]
                            .meta
                            .open_sub_file(files[index].name)
                            .unwrap();
                    }
                    5 => {
                        // new_sub_file
                        let parent = rng.gen_range(0, dirs.len());
                        let name = loop {
                            let name: [u8; 16] = rng.gen();
                            if !dirs[parent].sub_file_name.contains(&name) {
                                break name;
                            }
                        };
                        match dirs[parent].meta.new_sub_file(
                            name,
                            SaveFile {
                                padding1: 0,
                                block: 0,
                                size: 0,
                                padding2: 0,
                                next: 0,
                            },
                        ) {
                            Err(Error::NoSpace) => assert_eq!(files.len(), file_entry_count - 1),
                            Ok(meta) => {
                                assert!(files.len() < file_entry_count - 1);
                                assert!(dirs[parent].sub_file_name.insert(name));
                                files.push(File { meta, name, parent })
                            }
                            _ => unreachable!(),
                        }
                    }
                    6 => {
                        // delete file
                        if files.is_empty() {
                            continue;
                        }
                        let index = rng.gen_range(0, files.len());
                        let file = files.remove(index);
                        file.meta.delete().unwrap();
                        let parent = file.parent;
                        assert!(dirs[parent].sub_file_name.remove(&file.name));
                    }
                    7 => {
                        // rename file
                        if files.is_empty() {
                            continue;
                        }
                        let index = rng.gen_range(0, files.len());

                        let parent = rng.gen_range(0, dirs.len());
                        let name = loop {
                            let name: [u8; 16] = rng.gen();
                            if !dirs[parent].sub_file_name.contains(&name) {
                                break name;
                            }
                        };

                        assert!(dirs[files[index].parent]
                            .sub_file_name
                            .remove(&files[index].name));

                        files[index].name = name;
                        files[index].parent = parent;
                        assert!(dirs[files[index].parent]
                            .sub_file_name
                            .insert(files[index].name));
                        files[index].meta.rename(&dirs[parent].meta, name).unwrap();
                    }

                    8 => {
                        // rename dir
                        if dirs.len() == 1 {
                            continue;
                        }
                        let index = rng.gen_range(1, dirs.len());

                        let parent = rng.gen_range(0, dirs.len());
                        if parent == index {
                            continue;
                        }

                        let name = loop {
                            let name: [u8; 16] = rng.gen();
                            if !dirs[parent].sub_file_name.contains(&name) {
                                break name;
                            }
                        };

                        let old_parent = dirs[index].parent;
                        let old_name = dirs[index].name;
                        assert!(dirs[old_parent].sub_dir_name.remove(&old_name));

                        dirs[index].name = name;
                        dirs[index].parent = parent;
                        assert!(dirs[parent].sub_dir_name.insert(name));
                        let (a, b) = borrow_mut_two(&mut dirs, index, parent);
                        a.meta.rename(&b.meta, name).unwrap();
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    #[test]
    fn meta_fuzz() {
        let mut rng = rand::thread_rng();

        #[derive(ByteStruct, PartialEq, Clone, Debug, Hash, Eq)]
        #[byte_struct_le]
        struct Key {
            v: u32,
        }

        #[derive(ByteStruct, PartialEq, Clone, Debug)]
        #[byte_struct_le]
        struct Info {
            v: u32,
        }

        for _ in 0..100 {
            let mut key_set: HashSet<Key> = HashSet::new();
            let entry_count = rng.gen_range(10, 1000);
            let buckets = rng.gen_range(10, 100);
            let hash = Rc::new(MemoryFile::new(vec![0; buckets * 4]));
            let table = Rc::new(MemoryFile::new(vec![0; entry_count * 16]));
            MetaTable::<Key, Info>::format(hash.as_ref(), table.as_ref(), entry_count).unwrap();
            let meta = MetaTable::<Key, Info>::new(hash, table).unwrap();
            #[derive(Clone)]
            struct Image {
                key: Key,
                info: Info,
                pos: u32,
            }
            let mut chains: Vec<Image> = vec![];
            let mut occupied = 1;

            for _ in 0..1000 {
                match rng.gen_range(0, 5) {
                    0 => {
                        // add
                        let key = loop {
                            let key = Key { v: rng.gen() };
                            if key_set.insert(key.clone()) {
                                break key;
                            }
                        };
                        let info = Info { v: rng.gen() };
                        match meta.add(key.clone(), info.clone()) {
                            Err(Error::NoSpace) => assert_eq!(occupied, entry_count),
                            Ok(pos) => {
                                chains.push(Image { key, info, pos });
                                occupied += 1;
                            }
                            _ => unreachable!(),
                        }
                    }
                    1 => {
                        if chains.is_empty() {
                            continue;
                        }
                        // remove
                        let image_i = rng.gen_range(0, chains.len());
                        meta.remove(chains[image_i].pos).unwrap();
                        key_set.remove(&chains[image_i].key);
                        chains.remove(image_i);
                        occupied -= 1;
                    }
                    2 => {
                        if chains.is_empty() {
                            continue;
                        }
                        // get
                        if rng.gen() {
                            let key = loop {
                                let key = Key { v: rng.gen() };
                                if !key_set.contains(&key) {
                                    break key;
                                }
                            };
                            match meta.get(&key) {
                                Err(Error::NotFound) => {}
                                _ => unreachable!(),
                            }
                        } else {
                            let image_i = rng.gen_range(0, chains.len());
                            let (info, pos) = meta.get(&chains[image_i].key).unwrap();
                            assert_eq!(info, chains[image_i].info);
                            assert_eq!(pos, chains[image_i].pos);
                        }
                    }
                    3 => {
                        if chains.is_empty() {
                            continue;
                        }
                        // get_at
                        let image_i = rng.gen_range(0, chains.len());
                        let (info, key) = meta.get_at(chains[image_i].pos).unwrap();
                        assert_eq!(info, chains[image_i].info);
                        assert_eq!(key, chains[image_i].key);
                    }
                    4 => {
                        if chains.is_empty() {
                            continue;
                        }
                        // set
                        let image_i = rng.gen_range(0, chains.len());
                        let info = Info { v: rng.gen() };
                        chains[image_i].info = info.clone();
                        meta.set(chains[image_i].pos, info).unwrap();
                    }
                    _ => unreachable!(),
                };
            }
        }
    }
}
