use crate::dpfs_level::DpfsLevel;
use crate::dual_file::DualFile;
use crate::error::*;
use crate::ivfc_level::IvfcLevel;
use crate::misc::*;
use crate::random_access_file::*;
use crate::sub_file::SubFile;
use byte_struct::*;
use log::*;
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

pub struct DifiPartitionParam {
    pub dpfs_level2_block_len: usize,
    pub dpfs_level3_block_len: usize,
    pub ivfc_level1_block_len: usize,
    pub ivfc_level2_block_len: usize,
    pub ivfc_level3_block_len: usize,
    pub ivfc_level4_block_len: usize,
    pub data_len: usize,
    pub external_ivfc_level4: bool,
}

impl DifiPartitionParam {
    pub fn get_align(&self) -> usize {
        *[
            self.dpfs_level2_block_len,
            self.dpfs_level3_block_len,
            self.ivfc_level1_block_len,
            self.ivfc_level2_block_len,
            self.ivfc_level3_block_len,
            self.ivfc_level4_block_len,
        ]
        .iter()
        .max()
        .unwrap()
    }

    #[cfg(test)]
    pub fn random() -> DifiPartitionParam {
        use rand::prelude::*;
        let mut rng = rand::thread_rng();
        DifiPartitionParam {
            dpfs_level2_block_len: 1 << rng.gen_range(1, 10),
            dpfs_level3_block_len: 1 << rng.gen_range(1, 10),
            ivfc_level1_block_len: 1 << rng.gen_range(6, 10),
            ivfc_level2_block_len: 1 << rng.gen_range(6, 10),
            ivfc_level3_block_len: 1 << rng.gen_range(6, 10),
            ivfc_level4_block_len: 1 << rng.gen_range(6, 10),
            data_len: rng.gen_range(1, 10_000),
            external_ivfc_level4: rng.gen(),
        }
    }
}

/// Implements `RandomAccessFile` layer for a DIFI partition.
///
/// A DIFI partition consists of a descriptor file and a partition file.
/// It implements fast data integrity checking and atomic operation by wrapping
/// multiple DPFS and IVFC layers.
pub struct DifiPartition {
    dpfs_level1: Rc<DualFile>,
    dpfs_level2: Rc<DpfsLevel>,
    dpfs_level3: Rc<DpfsLevel>,
    ivfc_level1: Rc<IvfcLevel>,
    ivfc_level2: Rc<IvfcLevel>,
    ivfc_level3: Rc<IvfcLevel>,
    ivfc_level4: Rc<IvfcLevel>,
}

struct DifiPartitionInfo {
    difi_header: DifiHeader,
    ivfc_descriptor: IvfcDescriptor,
    dpfs_descriptor: DpfsDescriptor,
    descriptor_len: usize,
    partition_len: usize,
}

impl DifiPartition {
    fn calculate_info(param: &DifiPartitionParam) -> DifiPartitionInfo {
        let ivfc_level4_len = param.data_len;
        let ivfc_level3_len = (divide_up(ivfc_level4_len, param.ivfc_level4_block_len)) * 0x20;
        let ivfc_level2_len = (divide_up(ivfc_level3_len, param.ivfc_level3_block_len)) * 0x20;
        let ivfc_level1_len = (divide_up(ivfc_level2_len, param.ivfc_level2_block_len)) * 0x20;
        let master_hash_len = (divide_up(ivfc_level1_len, param.ivfc_level1_block_len)) * 0x20;

        fn ivfc_align(offset: usize, len: usize, block_len: usize) -> usize {
            if len >= 4 * block_len {
                align_up(offset, block_len)
            } else {
                align_up(offset, 8)
            }
        }

        let ivfc_level1_offset = 0;
        let ivfc_level2_offset = ivfc_align(
            ivfc_level1_offset + ivfc_level1_len,
            ivfc_level2_len,
            param.ivfc_level2_block_len,
        );
        let ivfc_level3_offset = ivfc_align(
            ivfc_level2_offset + ivfc_level2_len,
            ivfc_level3_len,
            param.ivfc_level3_block_len,
        );
        let ivfc_level4_offset = ivfc_align(
            ivfc_level3_offset + ivfc_level3_len,
            ivfc_level4_len,
            param.ivfc_level4_block_len,
        );
        let ivfc_end = ivfc_level4_offset + ivfc_level4_len;

        let duplicate_data_len = if param.external_ivfc_level4 {
            ivfc_level4_offset
        } else {
            ivfc_end
        };

        let dpfs_level3_len = align_up(duplicate_data_len, param.dpfs_level3_block_len);
        let dpfs_level2_len = align_up(
            (1 + (dpfs_level3_len / param.dpfs_level3_block_len - 1) / 32) * 4,
            param.dpfs_level2_block_len,
        );
        let dpfs_level1_len = (1 + (dpfs_level2_len / param.dpfs_level2_block_len - 1) / 32) * 4;

        let dpfs_level1_offset = 0;
        let dpfs_level2_offset = dpfs_level1_offset + dpfs_level1_len * 2;
        let dpfs_level3_offset = align_up(
            dpfs_level2_offset + dpfs_level2_len * 2,
            param.dpfs_level3_block_len,
        );
        let dpfs_end = dpfs_level3_offset + dpfs_level3_len * 2;

        let (partition_len, external_ivfc_level4_offset) = if param.external_ivfc_level4 {
            let ivfc_level4_offset = align_up(dpfs_end, param.ivfc_level4_block_len);
            (ivfc_level4_offset + ivfc_level4_len, ivfc_level4_offset)
        } else {
            (dpfs_end, 0)
        };

        fn ilog(block_len: usize) -> u32 {
            (std::mem::size_of::<usize>() * 8) as u32 - block_len.leading_zeros() - 1
        }

        let dpfs_descriptor = DpfsDescriptor {
            magic: *b"DPFS",
            version: 0x10000,
            level1_offset: dpfs_level1_offset as u64,
            level1_size: dpfs_level1_len as u64,
            level1_block_log: 0,
            padding1: 0,
            level2_offset: dpfs_level2_offset as u64,
            level2_size: dpfs_level2_len as u64,
            level2_block_log: ilog(param.dpfs_level2_block_len),
            padding2: 0,
            level3_offset: dpfs_level3_offset as u64,
            level3_size: dpfs_level3_len as u64,
            level3_block_log: ilog(param.dpfs_level3_block_len),
            padding3: 0,
        };

        let ivfc_descriptor = IvfcDescriptor {
            magic: *b"IVFC",
            version: 0x20000,
            master_hash_size: master_hash_len as u64,
            level1_offset: ivfc_level1_offset as u64,
            level1_size: ivfc_level1_len as u64,
            level1_block_log: ilog(param.ivfc_level1_block_len),
            padding1: 0,
            level2_offset: ivfc_level2_offset as u64,
            level2_size: ivfc_level2_len as u64,
            level2_block_log: ilog(param.ivfc_level2_block_len),
            padding2: 0,
            level3_offset: ivfc_level3_offset as u64,
            level3_size: ivfc_level3_len as u64,
            level3_block_log: ilog(param.ivfc_level3_block_len),
            padding3: 0,
            level4_offset: ivfc_level4_offset as u64,
            level4_size: ivfc_level4_len as u64,
            level4_block_log: ilog(param.ivfc_level4_block_len),
            padding4: 0,
            ivfc_descritor_size: IvfcDescriptor::BYTE_LEN as u64,
        };

        let ivfc_descriptor_offset = DifiHeader::BYTE_LEN;
        let dpfs_descriptor_offset = ivfc_descriptor_offset + IvfcDescriptor::BYTE_LEN;
        let master_hash_offset = dpfs_descriptor_offset + DpfsDescriptor::BYTE_LEN;
        let descriptor_len = master_hash_offset + master_hash_len;

        let difi_header = DifiHeader {
            magic: *b"DIFI",
            version: 0x10000,
            ivfc_descriptor_offset: ivfc_descriptor_offset as u64,
            ivfc_descriptor_size: IvfcDescriptor::BYTE_LEN as u64,
            dpfs_descriptor_offset: dpfs_descriptor_offset as u64,
            dpfs_descriptor_size: DpfsDescriptor::BYTE_LEN as u64,
            partition_hash_offset: master_hash_offset as u64,
            partition_hash_size: master_hash_len as u64,
            external_ivfc_level4: param.external_ivfc_level4 as u8,
            dpfs_selector: 0,
            padding: 0,
            ivfc_level4_offset: external_ivfc_level4_offset as u64,
        };

        DifiPartitionInfo {
            difi_header,
            ivfc_descriptor,
            dpfs_descriptor,
            descriptor_len,
            partition_len,
        }
    }

    pub fn calculate_size(param: &DifiPartitionParam) -> (usize, usize) {
        let info = DifiPartition::calculate_info(param);
        (info.descriptor_len, info.partition_len)
    }

    pub fn format(
        descriptor: &dyn RandomAccessFile,
        param: &DifiPartitionParam,
    ) -> Result<(), Error> {
        let info = DifiPartition::calculate_info(param);
        let ivfc_descriptor_offset = info.difi_header.ivfc_descriptor_offset as usize;
        let dpfs_descriptor_offset = info.difi_header.dpfs_descriptor_offset as usize;
        write_struct(descriptor, 0, info.difi_header)?;
        write_struct(descriptor, ivfc_descriptor_offset, info.ivfc_descriptor)?;
        write_struct(descriptor, dpfs_descriptor_offset, info.dpfs_descriptor)?;

        Ok(())
    }

    pub fn new(
        descriptor: Rc<dyn RandomAccessFile>,
        partition: Rc<dyn RandomAccessFile>,
    ) -> Result<DifiPartition, Error> {
        let header: DifiHeader = read_struct(descriptor.as_ref(), 0)?;

        if header.magic != *b"DIFI" || header.version != 0x10000 {
            error!(
                "Unexpected DIFI magic {:?} {:X}",
                header.magic, header.version
            );
            return make_error(Error::MagicMismatch);
        }

        if header.ivfc_descriptor_size as usize != IvfcDescriptor::BYTE_LEN {
            error!(
                "Unexpected ivfc_descriptor_size {}",
                header.ivfc_descriptor_size
            );
            return make_error(Error::SizeMismatch);
        }
        let ivfc: IvfcDescriptor =
            read_struct(descriptor.as_ref(), header.ivfc_descriptor_offset as usize)?;
        if ivfc.magic != *b"IVFC" || ivfc.version != 0x20000 {
            error!("Unexpected IVFC magic {:?} {:X}", ivfc.magic, ivfc.version);
            return make_error(Error::MagicMismatch);
        }
        if header.partition_hash_size != ivfc.master_hash_size {
            error!(
                "Unexpected partition_hash_size {}",
                header.partition_hash_size
            );
            return make_error(Error::SizeMismatch);
        }

        if header.dpfs_descriptor_size as usize != DpfsDescriptor::BYTE_LEN {
            error!(
                "Unexpected dpfs_descriptor_size {}",
                header.dpfs_descriptor_size
            );
            return make_error(Error::SizeMismatch);
        }
        let dpfs: DpfsDescriptor =
            read_struct(descriptor.as_ref(), header.dpfs_descriptor_offset as usize)?;
        if dpfs.magic != *b"DPFS" || dpfs.version != 0x10000 {
            error!("Unexpected DPFS magic {:?} {:X}", dpfs.magic, dpfs.version);
            return make_error(Error::MagicMismatch);
        }

        let dpfs_level0 = Rc::new(SubFile::new(descriptor.clone(), 0x39, 1)?);

        let dpfs_level1_pair: [Rc<dyn RandomAccessFile>; 2] = [
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

        let dpfs_level2_pair: [Rc<dyn RandomAccessFile>; 2] = [
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

        let dpfs_level3_pair: [Rc<dyn RandomAccessFile>; 2] = [
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
    use crate::memory_file::MemoryFile;

    #[test]
    fn struct_size() {
        assert_eq!(DifiHeader::BYTE_LEN, 0x44);
        assert_eq!(IvfcDescriptor::BYTE_LEN, 0x78);
        assert_eq!(DpfsDescriptor::BYTE_LEN, 0x50);
    }

    #[test]
    fn fuzz() {
        use rand::distributions::Standard;
        use rand::prelude::*;

        let rng = rand::thread_rng();
        for _ in 0..10 {
            let param = DifiPartitionParam::random();
            let len = param.data_len;

            let (descriptor_len, partition_len) = DifiPartition::calculate_size(&param);
            let descriptor = Rc::new(MemoryFile::new(vec![0; descriptor_len]));
            let partition = Rc::new(MemoryFile::new(vec![0; partition_len]));

            DifiPartition::format(descriptor.as_ref(), &param).unwrap();
            let difi = DifiPartition::new(descriptor.clone(), partition.clone()).unwrap();
            let init: Vec<u8> = rng.sample_iter(&Standard).take(len).collect();
            difi.write(0, &init).unwrap();
            let plain = MemoryFile::new(init);

            crate::random_access_file::fuzzer(
                difi,
                |file| file,
                |file| file.commit().unwrap(),
                || DifiPartition::new(descriptor.clone(), partition.clone()).unwrap(),
                plain,
            );
        }
    }
}
