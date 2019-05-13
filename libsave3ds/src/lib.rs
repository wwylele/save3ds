mod aes_ctr_file;
mod difi_partition;
mod disa;
mod disk_file;
mod dpfs_level;
mod dual_file;
pub mod error;
mod fat;
mod fs;
mod ivfc_level;
mod key_engine;
mod memory_file;
mod random_access_file;
pub mod save_data;
mod sd;
mod signed_file;
mod sub_file;

use aes_ctr_file::AesCtrFile;
use disk_file::DiskFile;
use error::*;
use key_engine::*;
use save_data::*;
use sd::Sd;
use sha2::*;
use std::io::{Read, Seek, SeekFrom};
use std::path::*;
use std::rc::Rc;

fn hash_movable(key: [u8; 16]) -> String {
    let mut hasher = Sha256::new();
    hasher.input(&key);
    let hash = hasher.result();
    let mut result = String::new();
    for index in &[3, 2, 1, 0, 7, 6, 5, 4, 11, 10, 9, 8, 15, 14, 13, 12] {
        result.extend(format!("{:02x}", hash[*index]).chars());
    }
    result
}

pub struct Resource {
    sd: Option<Sd>,
    nand_path: Option<PathBuf>,
    key_x_sign: Option<[u8; 16]>,
    key_y: Option<[u8; 16]>,
}

impl Resource {
    pub fn new(
        boot9_path: Option<String>,
        movable_path: Option<String>,
        sd_path: Option<String>,
        nand_path: Option<String>,
    ) -> Result<Resource, Error> {
        let nand_path = nand_path.map(|s| Path::new(&s).to_owned());

        let (key_x_sign, key_x_dec) = if let Some(boot9) = boot9_path {
            let mut boot9 = std::fs::File::open(boot9)?;
            let mut key_x_sign = [0; 16];
            let mut key_x_dec = [0; 16];
            boot9.seek(SeekFrom::Start(0xD9E0))?;
            boot9.read_exact(&mut key_x_sign)?;
            boot9.read_exact(&mut key_x_dec)?;
            (Some(key_x_sign), Some(key_x_dec))
        } else {
            (None, None)
        };

        let movable = if let Some(nand_path) = &nand_path {
            Some(nand_path.join("private").join("movable.sed"))
        } else {
            movable_path.map(|s| Path::new(&s).to_owned())
        };

        let key_y = if let Some(movable) = movable {
            let mut key_y = [0; 16];
            let mut movable = std::fs::File::open(&movable)?;
            movable.seek(SeekFrom::Start(0x110))?;
            movable.read_exact(&mut key_y)?;
            Some(key_y)
        } else {
            None
        };

        let sd = if let (Some(sd), Some(x), Some(y)) = (sd_path, key_x_dec, key_y) {
            Some(Sd::new(&sd, x, y)?)
        } else {
            None
        };

        Ok(Resource {
            sd,
            nand_path,
            key_x_sign,
            key_y,
        })
    }

    pub fn open_sd_save(&self, id: u64) -> Result<Rc<SaveData>, Error> {
        let id_high = format!("{:08x}", id >> 32);
        let id_low = format!("{:08x}", id & 0xFFFF_FFFF);
        let sub_path = ["title", &id_high, &id_low, "data", "00000001.sav"];

        let dec_file = Rc::new(self.sd.as_ref().ok_or(Error::NoSd)?.open(&sub_path)?);

        SaveData::new(
            dec_file,
            SaveDataType::Sd(
                scramble(
                    self.key_x_sign.ok_or(Error::NoBoot9)?,
                    self.key_y.ok_or(Error::NoNand)?,
                ),
                id,
            ),
        )
    }

    pub fn open_nand_save(&self, id: u32) -> Result<Rc<SaveData>, Error> {
        let path = self
            .nand_path
            .as_ref()
            .ok_or(Error::NoNand)?
            .join("data")
            .join(hash_movable(self.key_y.ok_or(Error::NoNand)?))
            .join("sysdata")
            .join(format!("{:08x}", id))
            .join("00000000");
        let file = Rc::new(DiskFile::new(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)?,
        )?);

        SaveData::new(
            file,
            SaveDataType::Nand(
                scramble(
                    self.key_x_sign.ok_or(Error::NoBoot9)?,
                    self.key_y.ok_or(Error::NoNand)?,
                ),
                id,
            ),
        )
    }

    pub fn open_bare_save(&self, path: &str) -> Result<Rc<SaveData>, Error> {
        let file = Rc::new(DiskFile::new(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)?,
        )?);

        SaveData::new(file, SaveDataType::Bare)
    }
}
