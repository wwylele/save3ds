use crate::random_access_file::*;
use byte_struct::*;
use std::marker::PhantomData;
use std::rc::Rc;

#[derive(ByteStruct)]
#[byte_struct_le]
struct U32le {
    v: u32,
}

struct MetaTable<KeyType, InfoType> {
    hash: Rc<RandomAccessFile>,
    table: Rc<RandomAccessFile>,

    buckets: usize,

    entry_len: usize,
    eo_info: usize,
    eo_collision: usize,

    phantom_key: PhantomData<KeyType>,
    phantom_info: PhantomData<InfoType>,
}

impl<KeyType: ByteStruct + PartialEq, InfoType: ByteStruct> MetaTable<KeyType, InfoType> {
    fn new(
        hash: Rc<RandomAccessFile>,
        table: Rc<RandomAccessFile>,
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
}

pub trait ParentedKey: ByteStruct + PartialEq + Clone {
    type NameType;
    fn get_parent(&self) -> u32;
    fn get_name(&self) -> Self::NameType;
    fn new(parent: u32, name: Self::NameType) -> Self;
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
    pub fn new(
        dir_hash: Rc<RandomAccessFile>,
        dir_table: Rc<RandomAccessFile>,
        file_hash: Rc<RandomAccessFile>,
        file_table: Rc<RandomAccessFile>,
    ) -> Result<Rc<FsMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType>>, Error> {
        Ok(Rc::new(FsMeta {
            dirs: MetaTable::new(dir_hash, dir_table)?,
            files: MetaTable::new(file_hash, file_table)?,
        }))
    }
}

pub struct FileMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType> {
    key: FileKeyType,
    pos: u32,
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
        let (_, key) = fs.files.get_at(ino)?;
        Ok(FileMeta { key, pos: ino, fs })
    }

    pub fn get_parent_ino(&self) -> u32 {
        self.key.get_parent()
    }

    pub fn get_ino(&self) -> u32 {
        self.pos
    }

    pub fn get_info(&self) -> Result<FileInfoType, Error> {
        Ok(self.fs.files.get_at(self.pos)?.0)
    }

    pub fn set_info(&self, info: FileInfoType) -> Result<(), Error> {
        self.fs.files.set(self.pos, info)
    }

    pub fn delete(self) -> Result<(), Error> {
        let (self_info, _) = self.fs.files.get_at(self.pos)?;

        let parent_index = self.key.get_parent();
        let (mut parent, _) = self.fs.dirs.get_at(parent_index)?;
        let mut head_index = parent.get_sub_file();
        if head_index == self.pos {
            parent.set_sub_file(self_info.get_next());
            self.fs.dirs.set(parent_index, parent)?;
        } else {
            loop {
                assert!(head_index != 0);
                let (mut head, _) = self.fs.files.get_at(head_index)?;
                let next_index = head.get_next();
                if next_index == self.pos {
                    head.set_next(self_info.get_next());
                    self.fs.files.set(head_index, head)?;
                    break;
                }
                head_index = next_index;
            }
        }

        self.fs.files.remove(self.pos)?;

        Ok(())
    }
}

pub struct DirMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType> {
    key: DirKeyType,
    pos: u32,
    fs: Rc<FsMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType>>,
}

impl<
        DirKeyType: ParentedKey,
        DirInfoType: DirInfo,
        FileKeyType: ParentedKey,
        FileInfoType: FileInfo,
    > DirMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType>
{
    pub fn open_root(
        fs: Rc<FsMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType>>,
    ) -> Result<Self, Error> {
        let (_, key) = fs.dirs.get_at(1)?;
        Ok(DirMeta { key, pos: 1, fs })
    }

    pub fn open_ino(
        fs: Rc<FsMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType>>,
        ino: u32,
    ) -> Result<Self, Error> {
        let (_, key) = fs.dirs.get_at(ino)?;
        Ok(DirMeta { key, pos: ino, fs })
    }

    pub fn get_parent_ino(&self) -> u32 {
        self.key.get_parent()
    }

    pub fn get_ino(&self) -> u32 {
        self.pos
    }

    pub fn open_sub_dir(&self, name: DirKeyType::NameType) -> Result<Self, Error> {
        let key = DirKeyType::new(self.pos, name);
        let (_, pos) = self.fs.dirs.get(&key)?;
        Ok(DirMeta {
            key,
            pos,
            fs: self.fs.clone(),
        })
    }

    pub fn open_sub_file(
        &self,
        name: FileKeyType::NameType,
    ) -> Result<FileMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType>, Error> {
        let key = FileKeyType::new(self.pos, name);
        let (_, pos) = self.fs.files.get(&key)?;
        Ok(FileMeta {
            key,
            pos,
            fs: self.fs.clone(),
        })
    }

    pub fn list_sub_dir(&self) -> Result<Vec<(DirKeyType::NameType, u32)>, Error> {
        let (self_info, _) = self.fs.dirs.get_at(self.pos)?;
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
        let (self_info, _) = self.fs.dirs.get_at(self.pos)?;
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
        mut info: DirInfoType,
    ) -> Result<Self, Error> {
        let (mut self_info, _) = self.fs.dirs.get_at(self.pos)?;
        let key = DirKeyType::new(self.pos, name);
        info.set_next(self_info.get_sub_dir());
        info.set_sub_dir(0);
        info.set_sub_file(0);
        let pos = self.fs.dirs.add(key.clone(), info)?;
        self_info.set_sub_dir(pos);
        self.fs.dirs.set(self.pos, self_info.clone())?;
        Ok(DirMeta {
            key,
            pos,
            fs: self.fs.clone(),
        })
    }

    pub fn new_sub_file(
        &self,
        name: FileKeyType::NameType,
        mut info: FileInfoType,
    ) -> Result<FileMeta<DirKeyType, DirInfoType, FileKeyType, FileInfoType>, Error> {
        let (mut self_info, _) = self.fs.dirs.get_at(self.pos)?;
        let key = FileKeyType::new(self.pos, name);
        info.set_next(self_info.get_sub_file());
        let pos = self.fs.files.add(key.clone(), info)?;
        self_info.set_sub_file(pos);
        self.fs.dirs.set(self.pos, self_info.clone())?;
        Ok(FileMeta {
            key,
            pos,
            fs: self.fs.clone(),
        })
    }

    pub fn delete(self) -> Result<Option<Self>, Error> {
        let (self_info, _) = self.fs.dirs.get_at(self.pos)?;
        if self.pos == 1 {
            return make_error(Error::DeletingRoot);
        }
        if self_info.get_sub_dir() != 0 {
            return Ok(Some(self));
        }
        if self_info.get_sub_file() != 0 {
            return Ok(Some(self));
        }

        let parent_index = self.key.get_parent();
        let (mut parent, _) = self.fs.dirs.get_at(parent_index)?;
        let mut head_index = parent.get_sub_dir();
        if head_index == self.pos {
            parent.set_sub_dir(self_info.get_next());
            self.fs.dirs.set(parent_index, parent)?;
        } else {
            loop {
                assert!(head_index != 0);
                let (mut head, _) = self.fs.dirs.get_at(head_index)?;
                let next_index = head.get_next();
                if next_index == self.pos {
                    head.set_next(self_info.get_next());
                    self.fs.dirs.set(head_index, head)?;
                    break;
                }
                head_index = next_index;
            }
        }

        self.fs.dirs.remove(self.pos)?;

        Ok(None)
    }
}

#[derive(ByteStruct, Clone)]
#[byte_struct_le]
pub struct SaveDir {
    pub next: u32,
    pub sub_dir: u32,
    pub sub_file: u32,
    pub padding: u32,
}

#[derive(ByteStruct, Clone)]
#[byte_struct_le]
pub struct SaveFile {
    pub next: u32,
    pub padding1: u32,
    pub block: u32,
    pub size: u64,
    pub padding2: u32,
}

#[derive(ByteStruct, Clone, PartialEq)]
#[byte_struct_le]
pub struct SaveKey {
    parent: u32,
    name: [u8; 16],
}

impl FileInfo for SaveFile {
    fn set_next(&mut self, index: u32) {
        self.next = index;
    }
    fn get_next(&self) -> u32 {
        self.next
    }
}

impl DirInfo for SaveDir {
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

impl ParentedKey for SaveKey {
    type NameType = [u8; 16];
    fn get_name(&self) -> [u8; 16] {
        self.name
    }
    fn get_parent(&self) -> u32 {
        self.parent
    }
    fn new(parent: u32, name: [u8; 16]) -> SaveKey {
        SaveKey { parent, name }
    }
}

#[cfg(test)]
mod test {
    use crate::fs::*;
    use crate::memory_file::MemoryFile;
    use rand::prelude::*;
    use std::collections::HashSet;
    use std::iter::*;

    #[test]
    fn struct_size() {
        assert_eq!(SaveDir::BYTE_LEN, 16);
        assert_eq!(SaveFile::BYTE_LEN, 24);
    }

    #[allow(clippy::cyclomatic_complexity)]
    #[test]
    fn fs_fuzz() {
        let mut rng = rand::thread_rng();
        for _ in 0..100 {
            let dir_entry_count = rng.gen_range(10, 1000);
            let dir_buckets = rng.gen_range(10, 100);
            let dir_hash = Rc::new(MemoryFile::new(vec![0; dir_buckets * 4]));
            let dir_table = Rc::new(MemoryFile::new(vec![
                0;
                dir_entry_count
                    * (SaveDir::BYTE_LEN
                        + SaveKey::BYTE_LEN
                        + 4)
            ]));
            write_struct(dir_table.as_ref(), 0, U32le { v: 1 }).unwrap();
            write_struct(
                dir_table.as_ref(),
                4,
                U32le {
                    v: dir_entry_count as u32,
                },
            )
            .unwrap();

            {
                let meta = MetaTable::<SaveKey, SaveDir>::new(dir_hash.clone(), dir_table.clone())
                    .unwrap();
                meta.add(
                    SaveKey::new(0, [0; 16]),
                    SaveDir {
                        next: 0,
                        sub_dir: 0,
                        sub_file: 0,
                        padding: 0,
                    },
                )
                .unwrap();
            }

            let file_entry_count = rng.gen_range(10, 1000);
            let file_buckets = rng.gen_range(10, 100);
            let file_hash = Rc::new(MemoryFile::new(vec![0; file_buckets * 4]));
            let file_table = Rc::new(MemoryFile::new(vec![
                0;
                file_entry_count
                    * (SaveFile::BYTE_LEN
                        + SaveKey::BYTE_LEN
                        + 4)
            ]));

            write_struct(file_table.as_ref(), 0, U32le { v: 1 }).unwrap();
            write_struct(
                file_table.as_ref(),
                4,
                U32le {
                    v: file_entry_count as u32,
                },
            )
            .unwrap();

            let fs = FsMeta::<SaveKey, SaveDir, SaveKey, SaveFile>::new(
                dir_hash, dir_table, file_hash, file_table,
            )
            .unwrap();

            struct Dir {
                meta: DirMeta<SaveKey, SaveDir, SaveKey, SaveFile>,
                name: [u8; 16],
                parent: usize,
                sub_dir_name: HashSet<[u8; 16]>,
                sub_file_name: HashSet<[u8; 16]>,
            }

            struct File {
                meta: FileMeta<SaveKey, SaveDir, SaveKey, SaveFile>,
                name: [u8; 16],
                parent: usize,
            }

            let mut dirs = vec![Dir {
                meta: DirMeta::<SaveKey, SaveDir, SaveKey, SaveFile>::open_root(fs.clone())
                    .unwrap(),
                name: [0; 16],
                parent: 0xFFFF_FFFF,
                sub_dir_name: HashSet::new(),
                sub_file_name: HashSet::new(),
            }];

            let mut files: Vec<File> = vec![];

            for _ in 0..1000 {
                match rng.gen_range(0, 7) {
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
                            SaveDir {
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
                        match dir.meta.delete() {
                            Ok(None) => {
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
                            Ok(Some(meta)) => {
                                assert!(
                                    !dir.sub_dir_name.is_empty() || !dir.sub_file_name.is_empty()
                                );
                                dir.meta = meta;
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
            write_struct(table.as_ref(), 0, U32le { v: 1 }).unwrap();
            write_struct(
                table.as_ref(),
                4,
                U32le {
                    v: entry_count as u32,
                },
            )
            .unwrap();
            write_struct(table.as_ref(), 12, U32le { v: 0 }).unwrap();

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
