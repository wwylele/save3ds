use crate::aes_ctr_file::*;
use crate::error::*;
use crate::file_system::*;
use crate::random_access_file::*;
use crate::save_data::*;
use crate::wear_leveling::*;
use std::rc::Rc;

pub struct CartSaveData {
    wear_leveling: Rc<WearLeveling>,
    save_data: SaveData,
}

impl CartSaveData {
    pub fn new(
        file: Rc<dyn RandomAccessFile>,
        key: [u8; 16],
        key_cmac: [u8; 16],
        repeat_ctr: bool,
    ) -> Result<CartSaveData, Error> {
        let wear_leveling = Rc::new(WearLeveling::new(file)?);

        let save = Rc::new(AesCtrFile::new(
            wear_leveling.clone(),
            key,
            [0; 16],
            repeat_ctr,
        ));

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
        self.wear_leveling.commit()
    }

    fn stat(&self) -> Result<Stat, Error> {
        self.save_data.stat()
    }
}
