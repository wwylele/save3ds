use crate::random_access_file::*;
use byte_struct::*;
use std::marker::PhantomData;
use std::rc::Rc;

struct InfoEx<InfoType: ByteStruct> {
    info: InfoType,
    next: u32,
}

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
    eo_next: usize,
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

        let entry_len = KeyType::BYTE_LEN + InfoType::BYTE_LEN + 8;
        let eo_next = KeyType::BYTE_LEN;
        let eo_info = KeyType::BYTE_LEN + 4;
        let eo_collision = KeyType::BYTE_LEN + InfoType::BYTE_LEN + 4;

        Ok(MetaTable {
            hash,
            table,
            buckets,
            entry_len,
            eo_next,
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

    fn get(&self, key: &KeyType) -> Result<Option<(InfoEx<InfoType>, u32)>, Error> {
        let h = self.hash(key);
        let table = self.table.as_ref();
        let hash = self.hash.as_ref();
        let mut index = read_struct::<U32le>(hash, h * 4)?.v;
        while index != 0 {
            let entry_offset = index as usize * self.entry_len;
            let other_key: KeyType = read_struct(table, entry_offset)?;
            if *key == other_key {
                let info = InfoEx {
                    info: read_struct(table, entry_offset + self.eo_info)?,
                    next: read_struct::<U32le>(table, entry_offset + self.eo_next)?.v,
                };
                return Ok(Some((info, index)));
            }

            index = read_struct::<U32le>(table, entry_offset + self.eo_collision)?.v;
        }
        Ok(None)
    }

    fn get_at(&self, index: u32) -> Result<(InfoEx<InfoType>, KeyType), Error> {
        let entry_offset = index as usize * self.entry_len;
        let table = self.table.as_ref();
        let info = InfoEx {
            info: read_struct(table, entry_offset + self.eo_info)?,
            next: read_struct::<U32le>(table, entry_offset + self.eo_next)?.v,
        };
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
        let next = read_struct::<U32le>(table, entry_offset + self.eo_next)?.v;
        let collision = read_struct::<U32le>(table, entry_offset + self.eo_collision)?.v;

        // scan to find the potential previous entry and relink it to the next entry
        // note that this will scan dummy entry as well. This is only safe when the corresponding
        // "next" field in the dummy entry is unused or has an unreachable value
        let entry_count = read_struct::<U32le>(table, 0)?.v;
        for i in 1..entry_count {
            let other_offset = i as usize * self.entry_len;
            let other_next = read_struct::<U32le>(table, other_offset + self.eo_next)?.v;
            if other_next == index {
                write_struct(table, other_offset + self.eo_next, U32le { v: next })?;
                // break;
            }
        }

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

    fn add(&self, key: KeyType, info: InfoEx<InfoType>) -> Result<u32, Error> {
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
        write_struct(table, entry_offset + self.eo_next, U32le { v: info.next })?;
        write_struct(table, entry_offset + self.eo_info, info.info)?;
        write_struct(table, entry_offset + self.eo_collision, collistion)?;

        Ok(index)
    }
}

trait DirInfo {
    fn set_sub_dir(&mut self, sub_dir: u32);
    fn get_sub_dir(&self) -> u32;
    fn set_sub_file(&mut self, sub_dir: u32);
    fn get_sub_file(&self) -> u32;
}

#[derive(ByteStruct)]
#[byte_struct_le]
struct SaveDir {
    sub_dir: u32,
    file: u32,
    padding: u32,
}

#[derive(ByteStruct)]
#[byte_struct_le]
struct SaveFile {
    padding1: u32,
    block: u32,
    size: u64,
    padding2: u32,
}

#[cfg(test)]
mod test {
    use crate::fs::*;
    use crate::memory_file::MemoryFile;

    #[test]
    fn struct_size() {
        assert_eq!(SaveDir::BYTE_LEN, 12);
        assert_eq!(SaveFile::BYTE_LEN, 20);
    }

    #[test]
    fn meta_fuzz() {
        use rand::prelude::*;
        use std::collections::HashSet;

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

            let chain_count = rng.gen_range(1, 10);

            #[derive(Clone)]
            struct Image {
                key: Key,
                info: Info,
                pos: u32,
            }
            let mut chains: Vec<Vec<Image>> = vec![vec![]; chain_count];
            let mut occupied = 1;

            for _ in 0..1000 {
                match rng.gen_range(0, 5) {
                    0 => {
                        // add
                        let chain_i = rng.gen_range(0, chain_count);
                        let key = loop {
                            let key = Key { v: rng.gen() };
                            if key_set.insert(key.clone()) {
                                break key;
                            }
                        };
                        let info = Info { v: rng.gen() };
                        let info_ex = InfoEx::<Info> {
                            info: info.clone(),
                            next: chains[chain_i].first().map_or(0, |i| i.pos),
                        };
                        match meta.add(key.clone(), info_ex) {
                            Err(Error::NoSpace) => assert_eq!(occupied, entry_count),
                            Ok(pos) => {
                                chains[chain_i].insert(0, Image { key, info, pos });
                                occupied += 1;
                            }
                            _ => unreachable!(),
                        }
                    }
                    1 => {
                        // remove
                        let chain_i = rng.gen_range(0, chain_count);
                        if chains[chain_i].is_empty() {
                            continue;
                        }
                        let image_i = rng.gen_range(0, chains[chain_i].len());
                        meta.remove(chains[chain_i][image_i].pos).unwrap();
                        key_set.remove(&chains[chain_i][image_i].key);
                        chains[chain_i].remove(image_i);
                        occupied -= 1;
                    }
                    2 => {
                        // get
                        if rng.gen() {
                            let key = loop {
                                let key = Key { v: rng.gen() };
                                if !key_set.contains(&key) {
                                    break key;
                                }
                            };
                            assert!(meta.get(&key).unwrap().is_none());
                        } else {
                            let chain_i = rng.gen_range(0, chain_count);
                            if chains[chain_i].is_empty() {
                                continue;
                            }
                            let image_i = rng.gen_range(0, chains[chain_i].len());
                            let (info_ex, pos) =
                                meta.get(&chains[chain_i][image_i].key).unwrap().unwrap();
                            assert_eq!(info_ex.info, chains[chain_i][image_i].info);
                            assert_eq!(
                                info_ex.next,
                                chains[chain_i].get(image_i + 1).map_or(0, |i| i.pos)
                            );
                            assert_eq!(pos, chains[chain_i][image_i].pos);
                        }
                    }
                    3 => {
                        // get_at
                        let chain_i = rng.gen_range(0, chain_count);
                        if chains[chain_i].is_empty() {
                            continue;
                        }
                        let image_i = rng.gen_range(0, chains[chain_i].len());
                        let (info_ex, key) = meta.get_at(chains[chain_i][image_i].pos).unwrap();
                        assert_eq!(info_ex.info, chains[chain_i][image_i].info);
                        assert_eq!(
                            info_ex.next,
                            chains[chain_i].get(image_i + 1).map_or(0, |i| i.pos)
                        );
                        assert_eq!(key, chains[chain_i][image_i].key);
                    }
                    4 => {
                        // set
                        let chain_i = rng.gen_range(0, chain_count);
                        if chains[chain_i].is_empty() {
                            continue;
                        }
                        let image_i = rng.gen_range(0, chains[chain_i].len());
                        let info = Info { v: rng.gen() };
                        chains[chain_i][image_i].info = info.clone();
                        meta.set(chains[chain_i][image_i].pos, info).unwrap();
                    }
                    _ => unreachable!(),
                };
            }
        }
    }
}
