use crate::difi_partition::*;
use crate::dual_file::DualFile;
use crate::error::*;
use crate::ivfc_level::IvfcLevel;
use crate::misc::*;
use crate::random_access_file::*;
use crate::signed_file::*;
use crate::sub_file::SubFile;
use byte_struct::*;
use log::*;
use std::ops::Index;
use std::rc::Rc;

#[derive(ByteStruct)]
#[byte_struct_le]
struct DisaPartitionDescriptorInfo {
    offset: u64,
    size: u64,
}

#[derive(ByteStruct)]
#[byte_struct_le]
struct DisaPartitionInfo {
    offset: u64,
    size: u64,
}

#[derive(ByteStruct)]
#[byte_struct_le]
struct DisaHeader {
    magic: [u8; 4],
    version: u32,
    partition_count: u32,
    padding1: u32,
    secondary_table_offset: u64,
    primary_table_offset: u64,
    table_size: u64,
    partition_descriptor: [DisaPartitionDescriptorInfo; 2],
    partition: [DisaPartitionInfo; 2],
    active_table: u8,
}

/// DISA container format that contains one or two DIFI partitions.
pub struct Disa {
    header_file: Rc<dyn RandomAccessFile>,
    table_upper: Rc<DualFile>,
    table_lower: Rc<IvfcLevel>,
    partitions: Vec<Rc<DifiPartition>>,
}

struct DisaInfo {
    secondary_table_offset: usize,
    primary_table_offset: usize,
    table_len: usize,
    descriptor_a_offset: usize,
    descriptor_a_len: usize,
    partition_a_offset: usize,
    partition_a_len: usize,
    descriptor_b_offset: usize,
    descriptor_b_len: usize,
    partition_b_offset: usize,
    partition_b_len: usize,
    end: usize,
}

impl Disa {
    fn calculate_info(
        partition_a_param: &DifiPartitionParam,
        partition_b_param: Option<&DifiPartitionParam>,
    ) -> DisaInfo {
        let (descriptor_a_len, partition_a_len) = DifiPartition::calculate_size(partition_a_param);
        let (descriptor_b_len, partition_b_len) =
            partition_b_param.map_or((0, 0), DifiPartition::calculate_size);
        let descriptor_a_offset = 0;
        let (descriptor_b_offset, table_len) = if partition_b_param.is_some() {
            let descriptor_b_offset = align_up(descriptor_a_offset + descriptor_a_len, 8);
            let table_len = align_up(descriptor_b_offset + descriptor_b_len, 8);
            (descriptor_b_offset, table_len)
        } else {
            (0, descriptor_a_len)
            // yeah, table_len doesn't align up to 8 in this case
        };

        let secondary_table_offset = 0x200;
        let primary_table_offset = align_up(secondary_table_offset + table_len, 8);

        let partition_a_align = partition_a_param.get_align();
        let partition_a_offset = align_up(primary_table_offset + table_len, partition_a_align);

        let (partition_b_offset, end) = if let Some(partition_b_param) = partition_b_param {
            let partition_b_align = partition_b_param.get_align();
            let partition_b_offset =
                align_up(partition_a_offset + partition_a_len, partition_b_align);
            let end = partition_b_offset + partition_b_len;
            (partition_b_offset, end)
        } else {
            (0, partition_a_offset + partition_a_len)
        };
        DisaInfo {
            secondary_table_offset,
            primary_table_offset,
            table_len,
            descriptor_a_offset,
            descriptor_a_len,
            partition_a_offset,
            partition_a_len,
            descriptor_b_offset,
            descriptor_b_len,
            partition_b_offset,
            partition_b_len,
            end,
        }
    }

    pub fn calculate_size(
        partition_a_param: &DifiPartitionParam,
        partition_b_param: Option<&DifiPartitionParam>,
    ) -> usize {
        Disa::calculate_info(partition_a_param, partition_b_param).end
    }

    pub fn format(
        file: Rc<dyn RandomAccessFile>,
        signer: Option<(Box<dyn Signer>, [u8; 16])>,
        partition_a_param: &DifiPartitionParam,
        partition_b_param: Option<&DifiPartitionParam>,
    ) -> Result<(), Error> {
        file.write(0, &[0; 0x200])?;
        let header_file_bare = Rc::new(SubFile::new(file.clone(), 0x100, 0x100)?);
        let header_file: Rc<dyn RandomAccessFile> = match signer {
            None => header_file_bare,
            Some((signer, key)) => Rc::new(SignedFile::new_unverified(
                Rc::new(SubFile::new(file.clone(), 0, 0x10)?),
                header_file_bare,
                signer,
                key,
            )?),
        };

        let info = Disa::calculate_info(partition_a_param, partition_b_param);

        let header = DisaHeader {
            magic: *b"DISA",
            version: 0x40000,
            partition_count: partition_b_param.is_some() as u32 + 1,
            padding1: 0,
            secondary_table_offset: info.secondary_table_offset as u64,
            primary_table_offset: info.primary_table_offset as u64,
            table_size: info.table_len as u64,
            partition_descriptor: [
                DisaPartitionDescriptorInfo {
                    offset: info.descriptor_a_offset as u64,
                    size: info.descriptor_a_len as u64,
                },
                DisaPartitionDescriptorInfo {
                    offset: info.descriptor_b_offset as u64,
                    size: info.descriptor_b_len as u64,
                },
            ],
            partition: [
                DisaPartitionInfo {
                    offset: info.partition_a_offset as u64,
                    size: info.partition_a_len as u64,
                },
                DisaPartitionInfo {
                    offset: info.partition_b_offset as u64,
                    size: info.partition_b_len as u64,
                },
            ],
            active_table: 1,
        };

        write_struct(header_file.as_ref(), 0, header)?;

        let table = Rc::new(IvfcLevel::new(
            Rc::new(SubFile::new(header_file.clone(), 0x6C, 0x20)?),
            Rc::new(SubFile::new(
                file.clone(),
                info.secondary_table_offset,
                info.table_len,
            )?),
            info.table_len,
        )?);

        let descriptor_a = Rc::new(SubFile::new(
            table.clone(),
            info.descriptor_a_offset,
            info.descriptor_a_len,
        )?);

        DifiPartition::format(descriptor_a.as_ref(), partition_a_param)?;

        if let Some(partition_b_param) = partition_b_param {
            let descriptor_b = Rc::new(SubFile::new(
                table.clone(),
                info.descriptor_b_offset,
                info.descriptor_b_len,
            )?);
            DifiPartition::format(descriptor_b.as_ref(), partition_b_param)?;
        }

        table.commit()?;
        header_file.commit()?;
        Ok(())
    }

    pub fn new(
        file: Rc<dyn RandomAccessFile>,
        signer: Option<(Box<dyn Signer>, [u8; 16])>,
    ) -> Result<Disa, Error> {
        let header_file_bare = Rc::new(SubFile::new(file.clone(), 0x100, 0x100)?);
        let header_file: Rc<dyn RandomAccessFile> = match signer {
            None => header_file_bare,
            Some((signer, key)) => Rc::new(SignedFile::new(
                Rc::new(SubFile::new(file.clone(), 0, 0x10)?),
                header_file_bare,
                signer,
                key,
            )?),
        };

        let header: DisaHeader = read_struct(header_file.as_ref(), 0)?;
        if header.magic != *b"DISA" || header.version != 0x40000 {
            error!(
                "Unexpected DISA magic {:?} {:X}",
                header.magic, header.version
            );
            return make_error(Error::MagicMismatch);
        }
        if header.partition_count != 1 && header.partition_count != 2 {
            error!("Unexpected partition_count {}", header.partition_count);
            return make_error(Error::InvalidValue);
        }

        let table_selector = Rc::new(SubFile::new(header_file.clone(), 0x68, 1)?);

        let table_hash = Rc::new(SubFile::new(header_file.clone(), 0x6C, 0x20)?);

        let table_pair: [Rc<dyn RandomAccessFile>; 2] = [
            Rc::new(SubFile::new(
                file.clone(),
                header.primary_table_offset as usize,
                header.table_size as usize,
            )?),
            Rc::new(SubFile::new(
                file.clone(),
                header.secondary_table_offset as usize,
                header.table_size as usize,
            )?),
        ];

        let table_upper = Rc::new(DualFile::new(table_selector, table_pair)?);

        let table_lower = Rc::new(IvfcLevel::new(
            table_hash,
            table_upper.clone(),
            header.table_size as usize,
        )?);

        let mut partitions = Vec::with_capacity(header.partition_count as usize);
        for i in 0..header.partition_count as usize {
            let d = &header.partition_descriptor[i];
            let p = &header.partition[i];
            let descriptor = Rc::new(SubFile::new(
                table_lower.clone(),
                d.offset as usize,
                d.size as usize,
            )?);
            let partition = Rc::new(SubFile::new(
                file.clone(),
                p.offset as usize,
                p.size as usize,
            )?);
            partitions.push(Rc::new(DifiPartition::new(descriptor, partition)?));
        }

        Ok(Disa {
            header_file,
            table_upper,
            table_lower,
            partitions,
        })
    }

    pub fn commit(&self) -> Result<(), Error> {
        for partition in self.partitions.iter() {
            partition.commit()?;
        }
        self.table_lower.commit()?;
        self.table_upper.commit()?;
        self.header_file.commit()
    }

    pub fn partition_count(&self) -> usize {
        self.partitions.len()
    }
}

impl Index<usize> for Disa {
    type Output = Rc<DifiPartition>;
    fn index(&self, index: usize) -> &Rc<DifiPartition> {
        &self.partitions[index]
    }
}

#[cfg(test)]
mod test {
    use crate::disa::*;
    use crate::memory_file::MemoryFile;
    use crate::signed_file::test::SimpleSigner;
    use rand::distributions::Standard;
    use rand::prelude::*;

    #[test]
    fn struct_size() {
        assert_eq!(DisaHeader::BYTE_LEN, 0x69);
    }

    fn fuzz_one_file(
        raw_file: Rc<MemoryFile>,
        partition_index: usize,
        signer: Option<(Box<SimpleSigner>, [u8; 16])>,
    ) {
        let rng = rand::thread_rng();
        let disa = Disa::new(
            raw_file.clone(),
            signer
                .as_ref()
                .map(|(a, b)| (a.clone() as Box<dyn Signer>, *b)),
        )
        .unwrap();
        let partition = &disa[partition_index];
        let len = partition.len();
        let init: Vec<u8> = rng.sample_iter(&Standard).take(len).collect();
        partition.write(0, &init).unwrap();
        let plain = MemoryFile::new(init);

        crate::random_access_file::fuzzer(
            disa,
            |disa| disa[partition_index].as_ref(),
            |disa| disa.commit().unwrap(),
            || {
                Disa::new(
                    raw_file.clone(),
                    signer
                        .as_ref()
                        .map(|(a, b)| (a.clone() as Box<dyn Signer>, *b)),
                )
                .unwrap()
            },
            plain,
        );
    }

    #[test]
    fn fuzz_one_partition() {
        let mut rng = rand::thread_rng();
        for _ in 0..10 {
            let signer = Box::new(SimpleSigner::new());
            let key = rng.gen();
            let param = DifiPartitionParam::random();
            let outer_len = Disa::calculate_size(&param, None);
            let outer = Rc::new(MemoryFile::new(vec![0; outer_len]));
            Disa::format(outer.clone(), Some((signer.clone(), key)), &param, None).unwrap();
            fuzz_one_file(outer, 0, Some((signer.clone(), key)));
        }
    }
    #[test]
    fn fuzz_two_partition() {
        let mut rng = rand::thread_rng();
        for _ in 0..10 {
            let signer = Box::new(SimpleSigner::new());
            let key = rng.gen();
            let param_a = DifiPartitionParam::random();
            let param_b = DifiPartitionParam::random();
            let outer_len = Disa::calculate_size(&param_a, Some(&param_b));
            let outer = Rc::new(MemoryFile::new(vec![0; outer_len]));
            Disa::format(
                outer.clone(),
                Some((signer.clone(), key)),
                &param_a,
                Some(&param_b),
            )
            .unwrap();
            fuzz_one_file(outer.clone(), 0, Some((signer.clone(), key)));
            fuzz_one_file(outer, 1, Some((signer.clone(), key)));
        }
    }
}
