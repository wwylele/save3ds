mod aes_ctr_file;
mod byte_struct_common;
mod cart_save_data;
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
mod misc;
mod nand;
mod random_access_file;
pub mod save_data;
mod save_ext_common;
mod sd;
mod sd_nand_common;
mod signed_file;
mod sub_file;
mod wear_leveling;

use aes::block_cipher_trait::generic_array::GenericArray;
use aes::block_cipher_trait::*;
use aes::*;
use cart_save_data::*;
use db::*;
use disk_file::DiskFile;
use error::*;
use ext_data::*;
use key_engine::*;
use misc::*;
use nand::Nand;
use save_data::*;
use sd::Sd;
use sd_nand_common::*;
use sha2::*;
use std::io::{Read, Seek, SeekFrom};
use std::path::*;
use std::rc::Rc;

pub struct Resource {
    sd: Option<Rc<Sd>>,
    nand: Option<Rc<Nand>>,
    key_x_ncch: Option<[u8; 16]>,
    key_x_dec: Option<[u8; 16]>,
    key_sign: Option<[u8; 16]>,
    key_x_sign: Option<[u8; 16]>,
    key_db: Option<[u8; 16]>,
    id0: Option<String>,
    cart_id_short: Option<[u8; 8]>,
    cart_id_long: Option<[u8; 0x40]>,
    game_path: Option<String>,
    x2f_key_y: Option<[u8; 16]>,
}

impl Resource {
    pub fn new(
        boot9_path: Option<String>,
        movable_path: Option<String>,
        sd_path: Option<String>,
        nand_path: Option<String>,
        otp_path: Option<String>,
        priv_path: Option<String>,
        game_path: Option<String>,
        x2f_key_y: Option<[u8; 16]>,
    ) -> Result<Resource, Error> {
        let (key_x_ncch, key_x_sign, key_x_dec, key_otp, iv_otp, otp_salt, key_y_db) =
            if let Some(boot9) = boot9_path {
                let mut boot9 = std::fs::File::open(boot9)?;
                let mut key_x_ncch = [0; 16];
                let mut key_x_sign = [0; 16];
                let mut key_x_dec = [0; 16];
                let mut key_otp = [0; 16];
                let mut iv_otp = [0; 16];
                let mut otp_salt = [0; 36];
                let mut otp_salt_iv = [0; 16];
                let mut otp_salt_block = [0; 64];
                let mut key_y_db = [0; 16];
                boot9.seek(SeekFrom::Start(0xD9D0))?;
                boot9.read_exact(&mut key_x_ncch)?;
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
                    Some(key_x_ncch),
                    Some(key_x_sign),
                    Some(key_x_dec),
                    Some(key_otp),
                    Some(iv_otp),
                    Some((otp_salt, otp_salt_iv, otp_salt_block)),
                    Some(key_y_db),
                )
            } else {
                (None, None, None, None, None, None, None)
            };

        let movable = if let Some(nand_path) = &nand_path {
            Some(PathBuf::from(nand_path).join("private").join("movable.sed"))
        } else {
            movable_path.map(|s| Path::new(&s).to_owned())
        };

        let (cart_id_short, cart_id_long) = if let Some(priv_path) = priv_path {
            let mut priv_header = std::fs::File::open(priv_path)?;
            let mut cart_id_short = [0; 8];
            let mut cart_id_long = [0; 0x40];
            priv_header.read_exact(&mut cart_id_long)?;
            priv_header.read_exact(&mut cart_id_short)?;
            (Some(cart_id_short), Some(cart_id_long))
        } else {
            (None, None)
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

        let id0 = key_y.map(hash_movable);

        let key_sign = (|| Some(scramble(key_x_sign?, key_y?)))();

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
            let key_otp = key_otp.ok_or(Error::MissingBoot9)?;
            let mut iv_otp = iv_otp.ok_or(Error::MissingBoot9)?;
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

            let (otp_salt, mut otp_salt_iv, mut otp_salt_block) =
                otp_salt.ok_or(Error::MissingBoot9)?;
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

        let key_db = (|| Some(scramble(key_x_db?, key_y_db?)))();

        Ok(Resource {
            sd,
            nand,
            key_x_ncch,
            key_x_dec,
            key_sign,
            key_x_sign,
            key_db,
            id0,
            cart_id_short,
            cart_id_long,
            game_path,
            x2f_key_y,
        })
    }

    pub fn format_sd_ext(&self, id: u64, param: &ExtDataFormatParam) -> Result<(), Error> {
        ExtData::format(
            self.sd.as_ref().ok_or(Error::MissingSd)?.as_ref(),
            &["extdata"],
            id,
            self.key_sign.ok_or(Error::MissingBoot9)?,
            None,
            param,
        )
    }

    pub fn open_sd_ext(&self, id: u64, write: bool) -> Result<ExtData, Error> {
        ExtData::new(
            self.sd.as_ref().ok_or(Error::MissingSd)?.clone(),
            &["extdata"],
            id,
            self.key_sign.ok_or(Error::MissingBoot9)?,
            false,
            write,
        )
    }

    pub fn format_sd_save(
        &self,
        id: u64,
        param: &SaveDataFormatParam,
        len: usize,
    ) -> Result<(), Error> {
        let block_count = SaveData::calculate_capacity(param, len);
        if block_count == 0 {
            return make_error(Error::NoSpace);
        }

        let id_high = format!("{:08x}", id >> 32);
        let id_low = format!("{:08x}", id & 0xFFFF_FFFF);
        let sub_path = ["title", &id_high, &id_low, "data", "00000001.sav"];

        let sd = self.sd.as_ref().ok_or(Error::MissingSd)?;
        sd.create(&sub_path, len)?;
        let file = sd.open(&sub_path, true)?;

        SaveData::format(
            file,
            SaveDataType::Sd(self.key_sign.ok_or(Error::MissingBoot9)?, id),
            &param,
            block_count,
        )?;

        Ok(())
    }

    pub fn open_sd_save(&self, id: u64, write: bool) -> Result<SaveData, Error> {
        let id_high = format!("{:08x}", id >> 32);
        let id_low = format!("{:08x}", id & 0xFFFF_FFFF);
        let sub_path = ["title", &id_high, &id_low, "data", "00000001.sav"];

        let dec_file = self
            .sd
            .as_ref()
            .ok_or(Error::MissingSd)?
            .open(&sub_path, write)?;

        SaveData::new(
            dec_file,
            SaveDataType::Sd(self.key_sign.ok_or(Error::MissingBoot9)?, id),
        )
    }

    pub fn format_nand_save(
        &self,
        id: u32,
        param: &SaveDataFormatParam,
        len: usize,
    ) -> Result<(), Error> {
        let block_count = SaveData::calculate_capacity(param, len);
        if block_count == 0 {
            return make_error(Error::NoSpace);
        }

        let sub_path = [
            "data",
            self.id0.as_ref().ok_or(Error::MissingNand)?,
            "sysdata",
            &format!("{:08x}", id),
            "00000000",
        ];

        let nand = self.nand.as_ref().ok_or(Error::MissingNand)?;
        nand.create(&sub_path, len)?;
        let file = nand.open(&sub_path, true)?;

        SaveData::format(
            file,
            SaveDataType::Nand(self.key_sign.ok_or(Error::MissingBoot9)?, id),
            &param,
            block_count,
        )?;

        Ok(())
    }

    pub fn open_nand_save(&self, id: u32, write: bool) -> Result<SaveData, Error> {
        let file = self.nand.as_ref().ok_or(Error::MissingNand)?.open(
            &[
                "data",
                self.id0.as_ref().ok_or(Error::MissingNand)?,
                "sysdata",
                &format!("{:08x}", id),
                "00000000",
            ],
            write,
        )?;
        SaveData::new(
            file,
            SaveDataType::Nand(self.key_sign.ok_or(Error::MissingBoot9)?, id),
        )
    }

    pub fn format_nand_ext(&self, id: u64, param: &ExtDataFormatParam) -> Result<(), Error> {
        ExtData::format(
            self.nand.as_ref().ok_or(Error::MissingNand)?.as_ref(),
            &[
                "data",
                self.id0.as_ref().ok_or(Error::MissingNand)?,
                "extdata",
            ],
            id,
            self.key_sign.ok_or(Error::MissingBoot9)?,
            Some(1024 * 1024),
            param,
        )
    }

    pub fn open_nand_ext(&self, id: u64, write: bool) -> Result<ExtData, Error> {
        ExtData::new(
            self.nand.as_ref().ok_or(Error::MissingNand)?.clone(),
            &[
                "data",
                self.id0.as_ref().ok_or(Error::MissingNand)?,
                "extdata",
            ],
            id,
            self.key_sign.ok_or(Error::MissingBoot9)?,
            true,
            write,
        )
    }

    pub fn format_bare_save(
        &self,
        path: &str,
        param: &SaveDataFormatParam,
        len: usize,
    ) -> Result<(), Error> {
        let block_count = SaveData::calculate_capacity(param, len);
        if block_count == 0 {
            return make_error(Error::NoSpace);
        }

        std::fs::File::create(path)?.set_len(len as u64)?;

        let file = Rc::new(DiskFile::new(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)?,
        )?);

        SaveData::format(file, SaveDataType::Bare, &param, block_count)?;

        Ok(())
    }

    pub fn open_bare_save(&self, path: &str, write: bool) -> Result<SaveData, Error> {
        let file = Rc::new(DiskFile::new(
            std::fs::OpenOptions::new()
                .read(true)
                .write(write)
                .open(path)?,
        )?);

        SaveData::new(file, SaveDataType::Bare)
    }

    pub fn get_cart_save_key_y(&self) -> Result<([u8; 16], bool), Error> {
        let game = disk_file::DiskFile::new(std::fs::File::open(
            self.game_path.as_ref().ok_or(Error::MissingGame)?,
        )?)?;

        use byte_struct_common::*;
        use random_access_file::*;
        if read_struct::<Magic>(&game, 0x100)?.v != *b"NCSD" {
            return Err(Error::BrokenGame);
        }

        let mut cci_flags = [0; 8];
        game.read(0x188, &mut cci_flags)?;

        let cxi_offset = read_struct::<U32le>(&game, 0x120)?.v * 0x200;
        let cxi_len = read_struct::<U32le>(&game, 0x124)?.v * 0x200;
        let cxi = sub_file::SubFile::new(Rc::new(game), cxi_offset as usize, cxi_len as usize)?;

        if read_struct::<Magic>(&cxi, 0x100)?.v != *b"NCCH" {
            return Err(Error::BrokenGame);
        }

        let mut program_id = [0; 8];
        cxi.read(0x118, &mut program_id)?;

        let mut key_y_ncch = [0; 16];
        cxi.read(0, &mut key_y_ncch)?;
        let ncch_key =
            key_engine::scramble(self.key_x_ncch.ok_or(Error::MissingBoot9)?, key_y_ncch);

        let ncch_version = read_struct::<U16le>(&cxi, 0x112)?.v;
        let exefs_offset = read_struct::<U32le>(&cxi, 0x1A0)?.v * 0x200;

        // CTR calculation below should use partition ID.
        // But for cartridge exe this should be always the same as program ID
        let mut ctr_exheader = [0; 16];
        let mut ctr_exefs;
        if ncch_version == 0 || ncch_version == 2 {
            ctr_exheader[0..8].copy_from_slice(&program_id);
            ctr_exheader[0..8].reverse();
            ctr_exefs = ctr_exheader;
            ctr_exheader[8] = 1;
            ctr_exefs[8] = 2;
        } else if ncch_version == 1 {
            ctr_exheader[0..8].copy_from_slice(&program_id);
            ctr_exefs = ctr_exheader;
            ctr_exheader[12..16].copy_from_slice(&0x200u32.to_be_bytes());
            ctr_exefs[12..16].copy_from_slice(&exefs_offset.to_be_bytes())
        } else {
            return Err(Error::BrokenGame);
        }

        let cxi = Rc::new(cxi);

        let exheader = aes_ctr_file::AesCtrFile::new(
            Rc::new(sub_file::SubFile::new(cxi.clone(), 0x200, 0x800)?),
            ncch_key,
            ctr_exheader,
            false,
        );
        let mut exheader_signature = [0; 8];
        exheader.read(0x400, &mut exheader_signature)?;

        let exefs = aes_ctr_file::AesCtrFile::new(
            Rc::new(sub_file::SubFile::new(cxi, exefs_offset as usize, 0x200)?),
            ncch_key,
            ctr_exefs,
            false,
        );

        let mut exefs_hash = [0; 32];
        let mut found = false;
        for i in 0..10 {
            let mut name = [0; 8];
            exefs.read(i * 0x10, &mut name)?;
            if name == *b".code\0\0\0" {
                found = true;
                exefs.read((9 - i) * 0x20 + 0xC0, &mut exefs_hash)?;
            }
        }
        if !found {
            return Err(Error::BrokenGame);
        }

        let crypto_version = if cci_flags[7] != 0 {
            2
        } else if cci_flags[3] != 0 {
            match cci_flags[1] {
                0 => 2,
                1 => 6,
                10 => 9,
                _ => return Err(Error::BrokenGame),
            }
        } else {
            0
        };

        let mut key_y = [0; 16];
        let mut repeat_ctr = false;
        match crypto_version {
            0 => {
                // TODO: version 0 is unverified yet
                let mut key_y_block = vec![];
                key_y_block.extend_from_slice(&exheader_signature);
                key_y_block.extend_from_slice(&self.cart_id_short.ok_or(Error::MissingPriv)?);
                key_y[..].copy_from_slice(&key_y_block[..]);
                repeat_ctr = true;
            }
            2 => {
                let mut key_y_block = vec![];
                key_y_block.extend_from_slice(&exheader_signature);
                key_y_block.extend_from_slice(&self.cart_id_long.ok_or(Error::MissingPriv)?);

                let mut hasher = Sha256::new();
                hasher.input(&key_y_block[..]);
                let hash = hasher.result();

                key_y[..].copy_from_slice(&hash[0..16]);
                // TODO: is there case where repeat_ctr = true for version 2?
            }
            6 => {
                let mut key_y_block = vec![];
                key_y_block.extend_from_slice(&exheader_signature);
                key_y_block.extend_from_slice(&self.cart_id_long.ok_or(Error::MissingPriv)?);
                key_y_block.extend_from_slice(&program_id);
                key_y_block.extend_from_slice(&exefs_hash);

                let mut hasher = Sha256::new();
                hasher.input(&key_y_block[..]);
                let hash = hasher.result();

                // Yup this one use the same key x as ncch
                let cmac_key = key_engine::scramble(
                    self.key_x_ncch.ok_or(Error::MissingBoot9)?,
                    self.x2f_key_y.ok_or(Error::MissingKeyY2F)?,
                );

                use cmac::*;
                let mut cmac = Cmac::<Aes128>::new(GenericArray::from_slice(&cmac_key[..]));
                cmac.input(&hash);
                key_y[..].copy_from_slice(cmac.result().code().as_slice());
            }
            // TODO: version 9
            _ => return Err(Error::Unsupported),
        }

        Ok((key_y, repeat_ctr))
    }

    pub fn open_cart_save(&self, path: &str, write: bool) -> Result<CartSaveData, Error> {
        let file = Rc::new(DiskFile::new(
            std::fs::OpenOptions::new()
                .read(true)
                .write(write)
                .open(path)?,
        )?);

        let (key_y, repeat_ctr) = self.get_cart_save_key_y()?;
        let key = key_engine::scramble(self.key_x_dec.ok_or(Error::MissingBoot9)?, key_y);
        let key_cmac = key_engine::scramble(self.key_x_sign.ok_or(Error::MissingBoot9)?, key_y);

        CartSaveData::new(file, key, key_cmac, repeat_ctr)
    }

    pub fn open_db(&self, db_type: DbType, write: bool) -> Result<Db, Error> {
        let (file, key) = match db_type {
            DbType::NandTitle => (
                self.nand
                    .as_ref()
                    .ok_or(Error::MissingNand)?
                    .open(&["dbs", "title.db"], write)?,
                self.key_db.ok_or(Error::MissingOtp)?,
            ),
            DbType::NandImport => (
                self.nand
                    .as_ref()
                    .ok_or(Error::MissingNand)?
                    .open(&["dbs", "import.db"], write)?,
                self.key_db.ok_or(Error::MissingOtp)?,
            ),
            DbType::TmpTitle => (
                self.nand
                    .as_ref()
                    .ok_or(Error::MissingNand)?
                    .open(&["dbs", "tmp_t.db"], write)?,
                self.key_db.ok_or(Error::MissingOtp)?,
            ),
            DbType::TmpImport => (
                self.nand
                    .as_ref()
                    .ok_or(Error::MissingNand)?
                    .open(&["dbs", "tmp_i.db"], write)?,
                self.key_db.ok_or(Error::MissingOtp)?,
            ),
            DbType::Ticket => (
                self.nand
                    .as_ref()
                    .ok_or(Error::MissingNand)?
                    .open(&["dbs", "ticket.db"], write)?,
                self.key_db.ok_or(Error::MissingOtp)?,
            ),
            DbType::SdTitle => (
                self.sd
                    .as_ref()
                    .ok_or(Error::MissingSd)?
                    .open(&["dbs", "title.db"], write)?,
                self.key_sign.ok_or(Error::MissingSd)?,
            ),
            DbType::SdImport => (
                self.sd
                    .as_ref()
                    .ok_or(Error::MissingSd)?
                    .open(&["dbs", "import.db"], write)?,
                self.key_sign.ok_or(Error::MissingSd)?,
            ),
        };

        Db::new(file, db_type, key)
    }
}
