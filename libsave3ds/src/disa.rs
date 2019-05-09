use crate::difi_partition::DifiPartition;
use crate::dual_file::DualFile;
use crate::error::*;
use crate::ivfc_level::IvfcLevel;
use crate::random_access_file::*;
use crate::sub_file::SubFile;
use byte_struct::*;
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

pub struct Disa {
    header_file: Rc<RandomAccessFile>,
    table_upper: Rc<DualFile>,
    table_lower: Rc<IvfcLevel>,
    partitions: Vec<Rc<DifiPartition>>,
}

impl Disa {
    pub fn new(file: Rc<RandomAccessFile>) -> Result<Disa, Error> {
        let header_file = Rc::new(SubFile::new(file.clone(), 0x100, 0x100)?); // TODO: link with CMAC
        let header: DisaHeader = read_struct(header_file.as_ref(), 0)?;
        if header.magic != *b"DISA" || header.version != 0x40000 {
            return make_error(Error::MagicMismatch);
        }
        if header.partition_count != 1 && header.partition_count != 2 {
            return make_error(Error::InvalidValue);
        }

        let table_selector = Rc::new(SubFile::new(header_file.clone(), 0x68, 1)?);

        let table_hash = Rc::new(SubFile::new(header_file.clone(), 0x6C, 0x20)?);

        let table_pair: [Rc<RandomAccessFile>; 2] = [
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

    #[test]
    fn struct_size() {
        assert_eq!(DisaHeader::BYTE_LEN, 0x69);
    }

    #[test]
    fn fuzz() {
        use crate::memory_file::MemoryFile;
        use rand::distributions::Standard;
        use rand::prelude::*;
        let mut rng = rand::thread_rng();

        let template = include_bytes!("00000000.disa");
        for _ in 0..10 {
            let raw_file = Rc::new(MemoryFile::new(template.to_vec()));
            let mut disa = Disa::new(raw_file.clone()).unwrap();
            let mut partition = &disa[0];
            let len = partition.len();
            let init: Vec<u8> = rng.sample_iter(&Standard).take(len).collect();
            partition.write(0, &init).unwrap();
            let plain = MemoryFile::new(init);

            for _ in 0..1000 {
                let operation = rng.gen_range(1, 10);
                if operation == 1 {
                    disa.commit().unwrap();
                    disa = Disa::new(raw_file.clone()).unwrap();
                    partition = &disa[0];
                } else if operation < 4 {
                    disa.commit().unwrap();
                } else {
                    let pos = rng.gen_range(0, len);
                    let data_len = rng.gen_range(1, len - pos + 1);
                    if operation < 7 {
                        let mut a = vec![0; data_len];
                        let mut b = vec![0; data_len];
                        partition.read(pos, &mut a).unwrap();
                        plain.read(pos, &mut b).unwrap();
                        assert_eq!(a, b);
                    } else {
                        let a: Vec<u8> = rng.sample_iter(&Standard).take(data_len).collect();
                        partition.write(pos, &a).unwrap();
                        plain.write(pos, &a).unwrap();
                    }
                }
            }
        }
    }

}
