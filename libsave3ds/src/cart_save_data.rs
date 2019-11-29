use crate::aes_ctr_file::*;
use crate::error::*;
use crate::file_system::*;
use crate::random_access_file::*;
use crate::save_data::*;
use crate::wear_leveling::*;
use std::rc::Rc;

pub(crate) struct CartFormat {
    pub wear_leveling: bool,
    pub key: [u8; 16],
    pub key_cmac: [u8; 16],
    pub repeat_ctr: bool,
}

/// A wrapper of [`SaveData`](../save_data/struct.SaveData.html),
/// specialized for cartridge save data. Implements [`FileSystem`](../file_system/trait.FileSystem.html).
pub struct CartSaveData {
    wear_leveling: Option<Rc<WearLeveling>>,
    save_data: SaveData,
}

impl CartSaveData {
    pub(crate) fn format(
        file: Rc<dyn RandomAccessFile>,
        &CartFormat {
            wear_leveling,
            key,
            key_cmac,
            repeat_ctr,
        }: &CartFormat,
        param: &SaveDataFormatParam,
    ) -> Result<(), Error> {
        let (wear_leveling, file): (_, Rc<dyn RandomAccessFile>) = if wear_leveling {
            Rc::new(WearLeveling::format(file.clone())?);
            let wear_leveling = Rc::new(WearLeveling::new(file)?);
            (Some(wear_leveling.clone()), wear_leveling)
        } else {
            (None, file)
        };

        let save = Rc::new(AesCtrFile::new(file, key, [0; 16], repeat_ctr));

        SaveData::format(save, SaveDataType::Cart(key_cmac), param)?;
        if let Some(wear_leveling) = wear_leveling {
            wear_leveling.commit()?;
        }
        Ok(())
    }

    pub(crate) fn new(
        file: Rc<dyn RandomAccessFile>,
        &CartFormat {
            wear_leveling,
            key,
            key_cmac,
            repeat_ctr,
        }: &CartFormat,
    ) -> Result<CartSaveData, Error> {
        let (wear_leveling, file): (_, Rc<dyn RandomAccessFile>) = if wear_leveling {
            let wear_leveling = Rc::new(WearLeveling::new(file)?);
            (Some(wear_leveling.clone()), wear_leveling)
        } else {
            (None, file)
        };

        let save = Rc::new(AesCtrFile::new(file, key, [0; 16], repeat_ctr));

        Ok(CartSaveData {
            wear_leveling,
            save_data: SaveData::new(save, SaveDataType::Cart(key_cmac))?,
        })
    }
}

impl FileSystem for CartSaveData {
    type FileType = <SaveData as FileSystem>::FileType;
    type DirType = <SaveData as FileSystem>::DirType;
    type NameType = <SaveData as FileSystem>::NameType;

    fn open_file(&self, ino: u32) -> Result<Self::FileType, Error> {
        self.save_data.open_file(ino)
    }

    fn open_dir(&self, ino: u32) -> Result<Self::DirType, Error> {
        self.save_data.open_dir(ino)
    }

    fn commit(&self) -> Result<(), Error> {
        self.save_data.commit()?;
        if let Some(wear_leveling) = &self.wear_leveling {
            wear_leveling.commit()?;
        }
        Ok(())
    }

    fn stat(&self) -> Result<Stat, Error> {
        self.save_data.stat()
    }
}

#[cfg(test)]
mod test {
    use crate::cart_save_data::*;

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
        use crate::memory_file::*;
        use rand::prelude::*;
        let mut rng = rand::thread_rng();

        for _ in 0..10 {
            let param = SaveDataFormatParam {
                block_type: match rng.gen_range(0, 2) {
                    0 => SaveDataBlockType::Small,
                    1 => SaveDataBlockType::Large,
                    _ => unreachable!(),
                },
                max_dir: rng.gen_range(10, 100),
                dir_buckets: rng.gen_range(10, 100),
                max_file: rng.gen_range(10, 100),
                file_buckets: rng.gen_range(10, 100),
                duplicate_data: rng.gen(),
            };

            let cart_format = CartFormat {
                wear_leveling: rng.gen(),
                key: rng.gen(),
                key_cmac: rng.gen(),
                repeat_ctr: rng.gen(),
            };

            let len = [0x20_000, 0x80_000, 0x100_000][rng.gen_range(0, 3)];
            let raw = Rc::new(MemoryFile::new(vec![0; len]));
            CartSaveData::format(raw.clone(), &cart_format, &param).unwrap();
            let file_system = CartSaveData::new(raw.clone(), &cart_format).unwrap();

            crate::file_system::test::fuzzer(
                file_system,
                param.max_dir as usize,
                param.max_file as usize,
                || CartSaveData::new(raw.clone(), &cart_format).unwrap(),
                gen_name,
                gen_len,
            );
        }
    }
}
