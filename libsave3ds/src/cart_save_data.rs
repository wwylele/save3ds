use crate::aes_ctr_file::*;
use crate::error::*;
use crate::file_system::*;
use crate::random_access_file::*;
use crate::save_data::*;
use crate::wear_leveling::*;
use std::rc::Rc;

pub struct CartFormat {
    pub wear_leveling: bool,
    pub key: [u8; 16],
    pub key_cmac: [u8; 16],
    pub repeat_ctr: bool,
}

pub struct CartSaveData {
    wear_leveling: Option<Rc<WearLeveling>>,
    save_data: SaveData,
}

impl CartSaveData {
    pub fn format(
        file: Rc<dyn RandomAccessFile>,
        CartFormat {
            wear_leveling,
            key,
            key_cmac,
            repeat_ctr,
        }: CartFormat,
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

    pub fn new(
        file: Rc<dyn RandomAccessFile>,
        CartFormat {
            wear_leveling,
            key,
            key_cmac,
            repeat_ctr,
        }: CartFormat,
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
