use crate::difi_partition::DifiPartition;
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

impl Diff {
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

    #[test]
    fn struct_size() {
        assert_eq!(DiffHeader::BYTE_LEN, 0x5C);
    }

}
