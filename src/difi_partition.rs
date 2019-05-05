use crate::dpfs_level::DpfsLevel;
use crate::dual_file::DualFile;
use crate::ivfc_level::IvfcLevel;
use crate::random_access_file::*;
use crate::sub_file::SubFile;
use byte_struct::*;
use std::rc::Rc;

#[derive(ByteStruct)]
#[byte_struct_le]
struct DifiHeader {
    magic: [u8; 4],
    version: u32,
    ivfc_descriptor_offset: u64,
    ivfc_descriptor_size: u64,
    dpfs_descriptor_offset: u64,
    dpfs_descriptor_size: u64,
    partition_hash_offset: u64,
    partition_hash_size: u64,
    external_ivfc_level4: u8,
    dpfs_selector: u8,
    padding: u16,
    ivfc_level4_offset: u64,
}

#[derive(ByteStruct)]
#[byte_struct_le]
struct IvfcDescriptor {
    magic: [u8; 4],
    version: u32,
    master_hash_size: u64,
    level1_offset: u64,
    level1_size: u64,
    level1_block_log: u32,
    padding1: u32,
    level2_offset: u64,
    level2_size: u64,
    level2_block_log: u32,
    padding2: u32,
    level3_offset: u64,
    level3_size: u64,
    level3_block_log: u32,
    padding3: u32,
    level4_offset: u64,
    level4_size: u64,
    level4_block_log: u32,
    padding4: u32,
    ivfc_descritor_size: u64,
}

#[derive(ByteStruct)]
#[byte_struct_le]
struct DpfsDescriptor {
    magic: [u8; 4],
    version: u32,
    level1_offset: u64,
    level1_size: u64,
    level1_block_log: u32,
    padding1: u32,
    level2_offset: u64,
    level2_size: u64,
    level2_block_log: u32,
    padding2: u32,
    level3_offset: u64,
    level3_size: u64,
    level3_block_log: u32,
    padding3: u32,
}

pub struct DifiPartition {
    dpfs_level1: Rc<DualFile>,
    dpfs_level2: Rc<DpfsLevel>,
    dpfs_level3: Rc<DpfsLevel>,
    ivfc_level1: Rc<IvfcLevel>,
    ivfc_level2: Rc<IvfcLevel>,
    ivfc_level3: Rc<IvfcLevel>,
    ivfc_level4: Rc<IvfcLevel>,
}

impl DifiPartition {
    pub fn new(
        descriptor: Rc<RandomAccessFile>,
        partition: Rc<RandomAccessFile>,
    ) -> Result<DifiPartition, Error> {
        let header: DifiHeader = descriptor.read_struct(0)?;

        if header.magic != *b"DIFI" || header.version != 0x10000 {
            return make_error(Error::MagicMismatch);
        }

        if header.ivfc_descriptor_size as usize != IvfcDescriptor::BYTE_LEN {
            return make_error(Error::SizeMismatch);
        }
        let ivfc: IvfcDescriptor =
            descriptor.read_struct(header.ivfc_descriptor_offset as usize)?;
        if ivfc.magic != *b"IVFC" || ivfc.version != 0x20000 {
            return make_error(Error::MagicMismatch);
        }
        if header.partition_hash_size != ivfc.master_hash_size {
            return make_error(Error::SizeMismatch);
        }

        if header.dpfs_descriptor_size as usize != DpfsDescriptor::BYTE_LEN {
            return make_error(Error::SizeMismatch);
        }
        let dpfs: DpfsDescriptor =
            descriptor.read_struct(header.dpfs_descriptor_offset as usize)?;
        if dpfs.magic != *b"DPFS" || dpfs.version != 0x10000 {
            return make_error(Error::MagicMismatch);
        }

        let dpfs_level0 = Rc::new(SubFile::new(descriptor.clone(), 0x39, 1)?);

        let dpfs_level1_pair: [Rc<RandomAccessFile>; 2] = [
            Rc::new(SubFile::new(
                partition.clone(),
                dpfs.level1_offset as usize,
                dpfs.level1_size as usize,
            )?),
            Rc::new(SubFile::new(
                partition.clone(),
                (dpfs.level1_offset + dpfs.level1_size) as usize,
                dpfs.level1_size as usize,
            )?),
        ];

        let dpfs_level2_pair: [Rc<RandomAccessFile>; 2] = [
            Rc::new(SubFile::new(
                partition.clone(),
                dpfs.level2_offset as usize,
                dpfs.level2_size as usize,
            )?),
            Rc::new(SubFile::new(
                partition.clone(),
                (dpfs.level2_offset + dpfs.level2_size) as usize,
                dpfs.level2_size as usize,
            )?),
        ];

        let dpfs_level3_pair: [Rc<RandomAccessFile>; 2] = [
            Rc::new(SubFile::new(
                partition.clone(),
                dpfs.level3_offset as usize,
                dpfs.level3_size as usize,
            )?),
            Rc::new(SubFile::new(
                partition.clone(),
                (dpfs.level3_offset + dpfs.level3_size) as usize,
                dpfs.level3_size as usize,
            )?),
        ];

        let dpfs_level1 = Rc::new(DualFile::new(dpfs_level0, dpfs_level1_pair)?);

        let dpfs_level2 = Rc::new(DpfsLevel::new(
            dpfs_level1.clone(),
            dpfs_level2_pair,
            1 << dpfs.level2_block_log,
        )?);

        let dpfs_level3 = Rc::new(DpfsLevel::new(
            dpfs_level2.clone(),
            dpfs_level3_pair,
            1 << dpfs.level3_block_log,
        )?);

        let ivfc_level0 = Rc::new(SubFile::new(
            descriptor.clone(),
            header.partition_hash_offset as usize,
            header.partition_hash_size as usize,
        )?);

        let ivfc_level1 = Rc::new(IvfcLevel::new(
            ivfc_level0,
            Rc::new(SubFile::new(
                dpfs_level3.clone(),
                ivfc.level1_offset as usize,
                ivfc.level1_size as usize,
            )?),
            1 << ivfc.level1_block_log,
        )?);

        let ivfc_level2 = Rc::new(IvfcLevel::new(
            ivfc_level1.clone(),
            Rc::new(SubFile::new(
                dpfs_level3.clone(),
                ivfc.level2_offset as usize,
                ivfc.level2_size as usize,
            )?),
            1 << ivfc.level2_block_log,
        )?);

        let ivfc_level3 = Rc::new(IvfcLevel::new(
            ivfc_level2.clone(),
            Rc::new(SubFile::new(
                dpfs_level3.clone(),
                ivfc.level3_offset as usize,
                ivfc.level3_size as usize,
            )?),
            1 << ivfc.level3_block_log,
        )?);

        let ivfc_level4 = Rc::new(IvfcLevel::new(
            ivfc_level3.clone(),
            Rc::new(if header.external_ivfc_level4 == 0 {
                SubFile::new(
                    dpfs_level3.clone(),
                    ivfc.level4_offset as usize,
                    ivfc.level4_size as usize,
                )?
            } else {
                SubFile::new(
                    partition.clone(),
                    header.ivfc_level4_offset as usize,
                    ivfc.level4_size as usize,
                )?
            }),
            1 << ivfc.level4_block_log,
        )?);

        Ok(DifiPartition {
            dpfs_level1,
            dpfs_level2,
            dpfs_level3,
            ivfc_level1,
            ivfc_level2,
            ivfc_level3,
            ivfc_level4,
        })
    }
}

impl RandomAccessFile for DifiPartition {
    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error> {
        self.ivfc_level4.read(pos, buf)
    }
    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error> {
        self.ivfc_level4.write(pos, buf)
    }
    fn len(&self) -> usize {
        self.ivfc_level4.len()
    }
    fn commit(&self) -> Result<(), Error> {
        self.ivfc_level4.commit()?;
        self.ivfc_level3.commit()?;
        self.ivfc_level2.commit()?;
        self.ivfc_level1.commit()?;
        self.dpfs_level3.commit()?;
        self.dpfs_level2.commit()?;
        self.dpfs_level1.commit()
    }
}

#[cfg(test)]
mod test {
    use crate::difi_partition::*;

    #[test]
    fn struct_size() {
        assert_eq!(DifiHeader::BYTE_LEN, 0x44);
        assert_eq!(IvfcDescriptor::BYTE_LEN, 0x78);
        assert_eq!(DpfsDescriptor::BYTE_LEN, 0x50);
    }

}
