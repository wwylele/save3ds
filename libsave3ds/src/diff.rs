use crate::align_up;
use crate::difi_partition::*;
use crate::dual_file::DualFile;
use crate::error::*;
use crate::ivfc_level::IvfcLevel;
use crate::random_access_file::*;
use crate::signed_file::*;
use crate::sub_file::SubFile;
use byte_struct::*;
use std::rc::Rc;

#[derive(ByteStruct)]
#[byte_struct_le]
struct DiffHeader {
    magic: [u8; 4],
    version: u32,
    secondary_table_offset: u64,
    primary_table_offset: u64,
    table_size: u64,
    partition_offset: u64,
    partition_size: u64,
    active_table: u8,
    padding: [u8; 3],
    sha: [u8; 0x20],
    unique_id: u64,
}

pub struct Diff {
    header_file: Rc<RandomAccessFile>,
    table_upper: Rc<DualFile>,
    table_lower: Rc<IvfcLevel>,
    partition: Rc<DifiPartition>,
    unique_id: u64,
}

struct DiffInfo {
    secondary_table_offset: usize,
    primary_table_offset: usize,
    table_size: usize,
    partition_offset: usize,
    partition_len: usize,
    end: usize,
}

impl Diff {
    fn calculate_info(param: &DifiPartitionParam) -> DiffInfo {
        let (descriptor_len, partition_len) = DifiPartition::calculate_size(param);
        let max_align = *[
            param.dpfs_level2_block_len,
            param.dpfs_level3_block_len,
            param.ivfc_level1_block_len,
            param.ivfc_level2_block_len,
            param.ivfc_level3_block_len,
            param.ivfc_level4_block_len,
        ]
        .iter()
        .max()
        .unwrap();
        let secondary_table_offset = 0x200;
        let primary_table_offset = align_up(secondary_table_offset + descriptor_len, 8);
        let table_size = descriptor_len;
        let partition_offset = align_up(primary_table_offset + descriptor_len, max_align);
        let end = partition_offset + partition_len;
        DiffInfo {
            secondary_table_offset,
            primary_table_offset,
            table_size,
            partition_offset,
            partition_len,
            end,
        }
    }

    pub fn calculate_size(param: &DifiPartitionParam) -> usize {
        Diff::calculate_info(param).end
    }

    pub fn format(
        file: Rc<RandomAccessFile>,
        signer: Option<(Box<Signer>, [u8; 16])>,
        param: &DifiPartitionParam,
        unique_id: u64,
    ) -> Result<(), Error> {
        file.write(0, &[0; 0x200])?;
        let header_file_bare = Rc::new(SubFile::new(file.clone(), 0x100, 0x100)?);
        let header_file: Rc<RandomAccessFile> = match signer {
            None => header_file_bare,
            Some((signer, key)) => Rc::new(SignedFile::new_unverified(
                Rc::new(SubFile::new(file.clone(), 0, 0x10)?),
                header_file_bare,
                signer,
                key,
            )?),
        };

        let info = Diff::calculate_info(param);

        let header = DiffHeader {
            magic: *b"DIFF",
            version: 0x30000,
            secondary_table_offset: info.secondary_table_offset as u64,
            primary_table_offset: info.primary_table_offset as u64,
            table_size: info.table_size as u64,
            partition_offset: info.partition_offset as u64,
            partition_size: info.partition_len as u64,
            active_table: 1,
            padding: [0; 3],
            sha: [0; 0x20],
            unique_id: unique_id,
        };

        write_struct(header_file.as_ref(), 0, header)?;

        let table = Rc::new(IvfcLevel::new(
            Rc::new(SubFile::new(header_file.clone(), 0x34, 0x20)?),
            Rc::new(SubFile::new(
                file.clone(),
                info.secondary_table_offset,
                info.table_size,
            )?),
            info.table_size,
        )?);

        DifiPartition::format(table.as_ref(), param)?;
        table.commit()?;
        header_file.commit()?;
        Ok(())
    }

    pub fn new(
        file: Rc<RandomAccessFile>,
        signer: Option<(Box<Signer>, [u8; 16])>,
    ) -> Result<Diff, Error> {
        let header_file_bare = Rc::new(SubFile::new(file.clone(), 0x100, 0x100)?);
        let header_file: Rc<RandomAccessFile> = match signer {
            None => header_file_bare,
            Some((signer, key)) => Rc::new(SignedFile::new(
                Rc::new(SubFile::new(file.clone(), 0, 0x10)?),
                header_file_bare,
                signer,
                key,
            )?),
        };

        let header: DiffHeader = read_struct(header_file.as_ref(), 0)?;
        if header.magic != *b"DIFF" || header.version != 0x30000 {
            return make_error(Error::MagicMismatch);
        }

        let table_selector = Rc::new(SubFile::new(header_file.clone(), 0x30, 1)?);

        let table_hash = Rc::new(SubFile::new(header_file.clone(), 0x34, 0x20)?);

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

        let partition = Rc::new(SubFile::new(
            file.clone(),
            header.partition_offset as usize,
            header.partition_size as usize,
        )?);
        let partition = Rc::new(DifiPartition::new(table_lower.clone(), partition)?);

        Ok(Diff {
            header_file,
            table_upper,
            table_lower,
            partition,
            unique_id: header.unique_id,
        })
    }

    pub fn commit(&self) -> Result<(), Error> {
        self.partition.commit()?;
        self.table_lower.commit()?;
        self.table_upper.commit()?;
        self.header_file.commit()
    }

    pub fn partition(&self) -> &Rc<DifiPartition> {
        &self.partition
    }

    pub fn unique_id(&self) -> u64 {
        self.unique_id
    }
}
#[cfg(test)]
mod test {
    use crate::diff::*;
    use crate::memory_file::MemoryFile;

    #[test]
    fn struct_size() {
        assert_eq!(DiffHeader::BYTE_LEN, 0x5C);
    }

    #[derive(Clone)]
    struct SimpleSigner {
        salt: u8,
    }

    impl Signer for SimpleSigner {
        fn block(&self, mut data: Vec<u8>) -> Vec<u8> {
            for b in data.iter_mut() {
                *b ^= self.salt;
            }
            data
        }
    }

    #[test]
    fn format_size() {
        let sample = include_str!("extdiffsize.txt");

        for line in sample.split('\n') {
            if line.is_empty() {
                continue;
            }
            let lr: Vec<_> = line.split(' ').collect();
            let left = lr[0].parse::<usize>().unwrap();
            let right = lr[1].parse::<usize>().unwrap();
            let param = DifiPartitionParam {
                dpfs_level2_block_len: 128,
                dpfs_level3_block_len: 4096,
                ivfc_level1_block_len: 512,
                ivfc_level2_block_len: 512,
                ivfc_level3_block_len: 4096,
                ivfc_level4_block_len: 4096,
                data_len: left,
                external_ivfc_level4: true,
            };
            assert_eq!(Diff::calculate_size(&param), right);
        }
    }

    #[test]
    fn fuzz() {
        use rand::distributions::Standard;
        use rand::prelude::*;

        let mut rng = rand::thread_rng();
        for _ in 0..10 {
            let len = rng.gen_range(1, 10_000);
            let signer = Box::new(SimpleSigner { salt: rng.gen() });
            let key = rng.gen();

            let param = DifiPartitionParam {
                dpfs_level2_block_len: 1 << rng.gen_range(1, 10),
                dpfs_level3_block_len: 1 << rng.gen_range(1, 10),
                ivfc_level1_block_len: 1 << rng.gen_range(6, 10),
                ivfc_level2_block_len: 1 << rng.gen_range(6, 10),
                ivfc_level3_block_len: 1 << rng.gen_range(6, 10),
                ivfc_level4_block_len: 1 << rng.gen_range(6, 10),
                data_len: len,
                external_ivfc_level4: rng.gen(),
            };

            let parent_len = Diff::calculate_size(&param);
            let parent = Rc::new(MemoryFile::new(vec![0; parent_len]));

            Diff::format(parent.clone(), Some((signer.clone(), key)), &param, 0).unwrap();
            let mut diff = Diff::new(parent.clone(), Some((signer.clone(), key))).unwrap();
            let init: Vec<u8> = rng.sample_iter(&Standard).take(len).collect();
            diff.partition().write(0, &init).unwrap();
            let plain = MemoryFile::new(init);

            crate::random_access_file::fuzzer(
                &mut diff,
                |diff| diff.partition().as_ref(),
                |diff| diff.commit().unwrap(),
                || Diff::new(parent.clone(), Some((signer.clone(), key))).unwrap(),
                &plain,
                len,
            );
        }
    }
}
