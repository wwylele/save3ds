mod aes_ctr_file;
pub mod db;
mod diff;
mod difi_partition;
mod disa;
mod disk_file;
mod dpfs_level;
mod dual_file;
pub mod error;
pub mod ext_data;
mod fat;
pub mod file_system;
mod fs_meta;
mod ivfc_level;
mod key_engine;
mod memory_file;
mod nand;
mod random_access_file;
pub mod save_data;
mod save_ext_common;
mod sd;
mod sd_nand_common;
mod signed_file;
mod sub_file;

use aes::block_cipher_trait::generic_array::GenericArray;
use aes::block_cipher_trait::*;
use aes::*;
use db::*;
use disk_file::DiskFile;
use error::*;
use ext_data::*;
use key_engine::*;
use nand::Nand;
use save_data::*;
use sd::Sd;
use sd_nand_common::*;
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
    sd: Option<Rc<Sd>>,
    nand: Option<Rc<Nand>>,
    key_x_sign: Option<[u8; 16]>,
    key_y: Option<[u8; 16]>,
    key_x_db: Option<[u8; 16]>,
    key_y_db: Option<[u8; 16]>,
}

impl Resource {
    pub fn new(
        boot9_path: Option<String>,
        movable_path: Option<String>,
        sd_path: Option<String>,
        nand_path: Option<String>,
        otp_path: Option<String>,
    ) -> Result<Resource, Error> {
        let (key_x_sign, key_x_dec, key_otp, iv_otp, otp_salt, key_y_db) =
            if let Some(boot9) = boot9_path {
                let mut boot9 = std::fs::File::open(boot9)?;
                let mut key_x_sign = [0; 16];
                let mut key_x_dec = [0; 16];
                let mut key_otp = [0; 16];
                let mut iv_otp = [0; 16];
                let mut otp_salt = [0; 36];
                let mut otp_salt_iv = [0; 16];
                let mut otp_salt_block = [0; 64];
                let mut key_y_db = [0; 16];
                boot9.seek(SeekFrom::Start(0xD9E0))?;
                boot9.read_exact(&mut key_x_sign)?;
                boot9.read_exact(&mut key_x_dec)?;
                boot9.seek(SeekFrom::Start(0xD6E0))?;
                boot9.read_exact(&mut key_otp)?;
                boot9.read_exact(&mut iv_otp)?;
                boot9.seek(SeekFrom::Start(0xD860))?;
                boot9.read_exact(&mut otp_salt)?;
                boot9.read_exact(&mut otp_salt_iv)?;
                boot9.read_exact(&mut otp_salt_block)?;
                boot9.seek(SeekFrom::Start(0xDAC0))?;
                boot9.read_exact(&mut key_y_db)?;
                (
                    Some(key_x_sign),
                    Some(key_x_dec),
                    Some(key_otp),
                    Some(iv_otp),
                    Some((otp_salt, otp_salt_iv, otp_salt_block)),
                    Some(key_y_db),
                )
            } else {
                (None, None, None, None, None, None)
            };

        let movable = if let Some(nand_path) = &nand_path {
            Some(PathBuf::from(nand_path).join("private").join("movable.sed"))
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
            Some(Rc::new(Sd::new(&sd, x, y)?))
        } else {
            None
        };

        let nand = if let Some(nand_path) = nand_path {
            Some(Rc::new(Nand::new(&nand_path)?))
        } else {
            None
        };

        let key_x_db = if let Some(otp_path) = otp_path {
            let key_otp = key_otp.ok_or(Error::NoBoot9)?;
            let mut iv_otp = iv_otp.ok_or(Error::NoBoot9)?;
            let mut otp_file = std::fs::File::open(otp_path)?;
            let mut otp = [0; 0x100];
            otp_file.read_exact(&mut otp)?;
            let aes128 = Aes128::new(GenericArray::from_slice(&key_otp));
            for block in otp.chunks_exact_mut(0x10) {
                let mut pad = [0; 16];
                pad.copy_from_slice(block);
                aes128.decrypt_block(GenericArray::from_mut_slice(block));
                for (i, b) in block.iter_mut().enumerate() {
                    *b ^= iv_otp[i];
                }
                iv_otp = pad;
            }

            let mut hasher = Sha256::new();
            hasher.input(&otp[0..0xE0]);
            if otp[0xE0..0x100] != hasher.result()[..] {
                return make_error(Error::BrokenOtp);
            }

            let (otp_salt, mut otp_salt_iv, mut otp_salt_block) = otp_salt.ok_or(Error::NoBoot9)?;
            let mut hasher = Sha256::new();
            hasher.input(&otp[0x90..0xAC]);
            hasher.input(&otp_salt[..]);
            let hash = hasher.result();
            let mut key_x = [0; 16];
            let mut key_y = [0; 16];
            key_x.copy_from_slice(&hash[0..16]);
            key_y.copy_from_slice(&hash[16..32]);
            let key = scramble(key_x, key_y);
            let aes128 = Aes128::new(GenericArray::from_slice(&key));

            for block in otp_salt_block.chunks_exact_mut(0x10) {
                for (i, b) in block.iter_mut().enumerate() {
                    *b ^= otp_salt_iv[i];
                }
                aes128.encrypt_block(GenericArray::from_mut_slice(block));
                otp_salt_iv.copy_from_slice(&block);
            }

            let mut key_x_db = [0; 16];
            key_x_db.copy_from_slice(&otp_salt_block[16..32]);
            Some(key_x_db)
        } else {
            None
        };

        Ok(Resource {
            sd,
            nand,
            key_x_sign,
            key_y,
            key_x_db,
            key_y_db,
        })
    }

    pub fn open_sd_ext(&self, id: u64, write: bool) -> Result<Rc<ExtData>, Error> {
        ExtData::new(
            self.sd.as_ref().ok_or(Error::NoSd)?.clone(),
            vec!["extdata".to_owned()],
            id,
            scramble(
                self.key_x_sign.ok_or(Error::NoBoot9)?,
                self.key_y.ok_or(Error::NoMovable)?,
            ),
            write,
        )
    }

    pub fn open_sd_save(&self, id: u64, write: bool) -> Result<Rc<SaveData>, Error> {
        let id_high = format!("{:08x}", id >> 32);
        let id_low = format!("{:08x}", id & 0xFFFF_FFFF);
        let sub_path = ["title", &id_high, &id_low, "data", "00000001.sav"];

        let dec_file = self
            .sd
            .as_ref()
            .ok_or(Error::NoSd)?
            .open(&sub_path, write)?;

        SaveData::new(
            dec_file,
            SaveDataType::Sd(
                scramble(
                    self.key_x_sign.ok_or(Error::NoBoot9)?,
                    self.key_y.ok_or(Error::NoMovable)?,
                ),
                id,
            ),
        )
    }

    pub fn open_nand_save(&self, id: u32, write: bool) -> Result<Rc<SaveData>, Error> {
        let file = self.nand.as_ref().ok_or(Error::NoNand)?.open(
            &[
                "data",
                &hash_movable(self.key_y.ok_or(Error::NoNand)?),
                "sysdata",
                &format!("{:08x}", id),
                "00000000",
            ],
            write,
        )?;
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

    pub fn open_nand_ext(&self, id: u64, write: bool) -> Result<Rc<ExtData>, Error> {
        ExtData::new(
            self.nand.as_ref().ok_or(Error::NoNand)?.clone(),
            vec![
                "data".to_owned(),
                hash_movable(self.key_y.ok_or(Error::NoNand)?),
                "extdata".to_owned(),
            ],
            id,
            scramble(
                self.key_x_sign.ok_or(Error::NoBoot9)?,
                self.key_y.ok_or(Error::NoMovable)?,
            ),
            write,
        )
    }

    pub fn open_bare_save(&self, path: &str, write: bool) -> Result<Rc<SaveData>, Error> {
        let file = Rc::new(DiskFile::new(
            std::fs::OpenOptions::new()
                .read(true)
                .write(write)
                .open(path)?,
        )?);

        SaveData::new(file, SaveDataType::Bare)
    }

    pub fn open_db(&self, db_type: DbType, write: bool) -> Result<Rc<Db>, Error> {
        let (file, key) = match db_type {
            DbType::NandTitle => (
                self.nand
                    .as_ref()
                    .ok_or(Error::NoNand)?
                    .open(&["dbs", "title.db"], write)?,
                Some(scramble(
                    self.key_x_db.ok_or(Error::NoOtp)?,
                    self.key_y_db.ok_or(Error::NoBoot9)?,
                )),
            ),
            DbType::NandImport => (
                self.nand
                    .as_ref()
                    .ok_or(Error::NoNand)?
                    .open(&["dbs", "import.db"], write)?,
                Some(scramble(
                    self.key_x_db.ok_or(Error::NoOtp)?,
                    self.key_y_db.ok_or(Error::NoBoot9)?,
                )),
            ),
            DbType::TmpTitle => (
                self.nand
                    .as_ref()
                    .ok_or(Error::NoNand)?
                    .open(&["dbs", "tmp_t.db"], write)?,
                Some(scramble(
                    self.key_x_db.ok_or(Error::NoOtp)?,
                    self.key_y_db.ok_or(Error::NoBoot9)?,
                )),
            ),
            DbType::TmpImport => (
                self.nand
                    .as_ref()
                    .ok_or(Error::NoNand)?
                    .open(&["dbs", "tmp_i.db"], write)?,
                Some(scramble(
                    self.key_x_db.ok_or(Error::NoOtp)?,
                    self.key_y_db.ok_or(Error::NoBoot9)?,
                )),
            ),
            DbType::Ticket => (
                self.nand
                    .as_ref()
                    .ok_or(Error::NoNand)?
                    .open(&["dbs", "ticket.db"], write)?,
                Some(scramble(
                    self.key_x_db.ok_or(Error::NoOtp)?,
                    self.key_y_db.ok_or(Error::NoBoot9)?,
                )),
            ),
            DbType::SdTitle => (
                self.sd
                    .as_ref()
                    .ok_or(Error::NoSd)?
                    .open(&["dbs", "title.db"], write)?,
                Some(scramble(
                    self.key_x_sign.ok_or(Error::NoBoot9)?,
                    self.key_y.ok_or(Error::NoMovable)?,
                )),
            ),
            DbType::SdImport => (
                self.sd
                    .as_ref()
                    .ok_or(Error::NoSd)?
                    .open(&["dbs", "import.db"], write)?,
                Some(scramble(
                    self.key_x_sign.ok_or(Error::NoBoot9)?,
                    self.key_y.ok_or(Error::NoMovable)?,
                )),
            ),
        };

        Db::new(file, db_type, key)
    }
}
