use crate::byte_struct_common::*;
use crate::error::*;
use crate::misc::*;
use crate::random_access_file::*;
use crate::sub_file::SubFile;
use std::collections::HashSet;
use std::rc::Rc;

pub fn crc16_ninty(data: &[u8]) -> u16 {
    let poly = 0xA001;
    let mut crc = 0xFFFFu16;
    for byte in data {
        crc ^= <u16>::from(*byte);
        for _ in 0..8 {
            let b = crc & 1 != 0;
            crc >>= 1;
            if b {
                crc ^= poly;
            }
        }
    }
    crc
}

trait CrcStub {
    fn verify(&self, crc: u16) -> Result<bool, Error>;
    fn sign(&self, crc: u16) -> Result<(), Error>;
}

struct SimpleCrcStub<F> {
    parent: Rc<F>,
}

impl<F: RandomAccessFile> SimpleCrcStub<F> {
    fn new(parent: Rc<F>) -> Result<SimpleCrcStub<F>, Error> {
        if parent.len() != 2 {
            return Err(Error::SizeMismatch);
        }
        Ok(SimpleCrcStub { parent })
    }
}

impl<F: RandomAccessFile> CrcStub for SimpleCrcStub<F> {
    fn verify(&self, crc: u16) -> Result<bool, Error> {
        Ok(read_struct::<U16le>(self.parent.as_ref(), 0)?.v == crc)
    }

    fn sign(&self, crc: u16) -> Result<(), Error> {
        write_struct(self.parent.as_ref(), 0, U16le { v: crc })
    }
}

struct XorCrcStub<F> {
    parent: Rc<F>,
}

impl<F: RandomAccessFile> XorCrcStub<F> {
    fn new(parent: Rc<F>) -> Result<XorCrcStub<F>, Error> {
        if parent.len() != 1 {
            return Err(Error::SizeMismatch);
        }
        Ok(XorCrcStub { parent })
    }
}

impl<F: RandomAccessFile> CrcStub for XorCrcStub<F> {
    fn verify(&self, crc: u16) -> Result<bool, Error> {
        let crc = crc.to_le_bytes();
        let crc = crc[0] ^ crc[1];
        let mut buf = [0];
        self.parent.read(0, &mut buf)?;
        Ok(buf[0] == crc)
    }

    fn sign(&self, crc: u16) -> Result<(), Error> {
        let crc = crc.to_le_bytes();
        let crc = [crc[0] ^ crc[1]];
        self.parent.write(0, &crc)
    }
}

struct CrcFile<C, F> {
    crc_stub: C,
    data: Rc<F>,
    len: usize,
}

impl<C: CrcStub, F: RandomAccessFile> CrcFile<C, F> {
    fn new(crc_stub: C, data: Rc<F>) -> Result<CrcFile<C, F>, Error> {
        let len = data.len();
        let mut buf = vec![0; len];
        data.read(0, &mut buf)?;
        if !crc_stub.verify(crc16_ninty(&buf))? {
            return Err(Error::SignatureMismatch);
        }
        Ok(CrcFile {
            crc_stub,
            data,
            len,
        })
    }
}

impl<C: CrcStub, F: RandomAccessFile> RandomAccessFile for CrcFile<C, F> {
    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error> {
        self.data.read(pos, buf)
    }
    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error> {
        self.data.write(pos, buf)
    }
    fn len(&self) -> usize {
        self.len
    }
    fn commit(&self) -> Result<(), Error> {
        let mut buf = vec![0; self.len];
        self.data.read(0, &mut buf)?;
        self.crc_stub.sign(crc16_ninty(&buf))
    }
}

struct MirroredFile<F0, F1> {
    data0: Rc<F0>,
    data1: Rc<F1>,
}

impl<F0: RandomAccessFile, F1: RandomAccessFile> MirroredFile<F0, F1> {
    fn new(data0: Rc<F0>, data1: Rc<F1>) -> Result<MirroredFile<F0, F1>, Error> {
        if data0.len() != data1.len() {
            return Err(Error::SizeMismatch);
        }
        let mut buf0 = vec![0; data0.len()];
        let mut buf1 = vec![0; data0.len()];
        data0.read(0, &mut buf0)?;
        data1.read(0, &mut buf1)?;
        if buf0 != buf1 {
            return Err(Error::SignatureMismatch);
        }
        Ok(MirroredFile { data0, data1 })
    }
}

impl<F0: RandomAccessFile, F1: RandomAccessFile> RandomAccessFile for MirroredFile<F0, F1> {
    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error> {
        self.data0.read(pos, buf)
    }
    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error> {
        self.data0.write(pos, buf)?;
        self.data1.write(pos, buf)
    }
    fn len(&self) -> usize {
        self.data0.len()
    }
    fn commit(&self) -> Result<(), Error> {
        Ok(())
    }
}

struct WearLevelingBlock {
    data: Option<Vec<CrcFile<XorCrcStub<SubFile>, SubFile>>>,
}

pub struct WearLeveling {
    blocks: Vec<WearLevelingBlock>,
}

impl WearLeveling {
    pub fn new(parent: Rc<dyn RandomAccessFile>) -> Result<WearLeveling, Error> {
        let len = parent.len();
        if len != 0x20000 && len != 0x80000 {
            return Err(Error::SizeMismatch);
        }
        let physical_block_count = len / 0x1000;
        let virtual_block_count = physical_block_count - 1;

        let mut unknown_header = vec![0; 8];
        parent.read(0, &mut unknown_header)?;

        let block_map = Rc::new(SubFile::new(
            parent.clone(),
            0,
            8 + virtual_block_count * 10,
        )?);
        let block_map_crc = Rc::new(SubFile::new(
            parent.clone(),
            8 + virtual_block_count * 10,
            2,
        )?);
        let block_map = Rc::new(CrcFile::new(SimpleCrcStub::new(block_map_crc)?, block_map)?);

        struct Block {
            physical_block: u8,
            allocate_count: u8,
            crc_ticket: Option<Rc<SubFile>>,
        };

        let mut blocks = vec![];
        for i in 0..virtual_block_count {
            let offset = i * 10 + 8;
            let mut buf = [0; 2];
            block_map.read(offset, &mut buf)?;
            blocks.push(Block {
                physical_block: buf[0] & 0x7F,
                allocate_count: buf[1],
                crc_ticket: if buf[0] & 0x80 != 0 {
                    Some(Rc::new(SubFile::new(block_map.clone(), offset + 2, 8)?))
                } else {
                    None
                },
            });
        }

        let mut physical_block_set: HashSet<_> = (1..physical_block_count).collect();
        for block in blocks.iter() {
            if !physical_block_set.remove(&(block.physical_block as usize)) {
                return Err(Error::InvalidValue);
            }
        }

        for offset in (8 + virtual_block_count * 10 + 2..0x1000).step_by(0x20) {
            let journal0 = Rc::new(SubFile::new(parent.clone(), offset, 14)?);
            let journal1 = Rc::new(SubFile::new(parent.clone(), offset + 14, 14)?);
            let journal = MirroredFile::new(journal0, journal1)?;
            let mut buf = [0; 6];
            journal.read(0, &mut buf)?;
            let virtual_block = buf[0] as usize;
            let virtual_block_prev = buf[1] as usize;
            let physical_block = buf[2];
            let physical_block_prev = buf[3];
            let allocate_count = buf[4];
            let allocate_count_prev = buf[5];
            if virtual_block == 0xFF {
                break;
            }

            if virtual_block >= virtual_block_count {
                return Err(Error::InvalidValue);
            }
            if virtual_block_prev >= virtual_block_count {
                return Err(Error::InvalidValue);
            }
            if physical_block as usize >= physical_block_count || physical_block == 0 {
                return Err(Error::InvalidValue);
            }
            if physical_block_prev as usize >= physical_block_count || physical_block_prev == 0 {
                return Err(Error::InvalidValue);
            }

            if blocks[virtual_block].physical_block != physical_block_prev {
                return Err(Error::InvalidValue);
            }

            if blocks[virtual_block_prev].physical_block != physical_block {
                return Err(Error::InvalidValue);
            }

            if blocks[virtual_block_prev].crc_ticket.is_some() {
                return Err(Error::InvalidValue);
            }

            if blocks[virtual_block].allocate_count != allocate_count_prev {
                return Err(Error::InvalidValue);
            }

            // Wrapping???
            if blocks[virtual_block_prev].allocate_count != allocate_count - 1 {
                return Err(Error::InvalidValue);
            }

            blocks[virtual_block_prev].allocate_count = allocate_count_prev;
            blocks[virtual_block_prev].physical_block = physical_block_prev;
            blocks[virtual_block_prev].crc_ticket = None;
            blocks[virtual_block].allocate_count = allocate_count;
            blocks[virtual_block].physical_block = physical_block;
            blocks[virtual_block].crc_ticket = Some(Rc::new(SubFile::new(Rc::new(journal), 6, 8)?));
        }

        if blocks.last().unwrap().crc_ticket.is_some() {
            return Err(Error::InvalidValue);
        }

        let mut final_blocks = vec![];
        for block in blocks {
            let data = if let Some(crc_ticket) = &block.crc_ticket {
                let mut data_list = vec![];
                for i in 0..8 {
                    let offset = i * 0x200 + block.physical_block as usize * 0x1000;
                    let data = Rc::new(SubFile::new(parent.clone(), offset, 0x200)?);
                    let crc = Rc::new(SubFile::new(crc_ticket.clone(), i, 1)?);
                    let data = CrcFile::new(XorCrcStub::new(crc)?, data)?;
                    data_list.push(data);
                }
                Some(data_list)
            } else {
                None
            };
            final_blocks.push(WearLevelingBlock { data });
        }

        Ok(WearLeveling {
            blocks: final_blocks,
        })
    }
}

impl RandomAccessFile for WearLeveling {
    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error> {
        let end = pos + buf.len();
        if end > self.len() {
            return Err(Error::OutOfBound);
        }

        // chunk index range the operation covers
        let begin_chunk = pos / 0x200;
        let end_chunk = divide_up(end, 0x200);

        for i in begin_chunk..end_chunk {
            // data range of this chunk
            let data_begin_as_chunk = i * 0x200;
            let data_end_as_chunk = (i + 1) * 0x200;

            // data range to read within this chunk
            let data_begin = std::cmp::max(data_begin_as_chunk, pos);
            let data_end = std::cmp::min(data_end_as_chunk, end);

            let block = i / 8;
            if let Some(block_data) = &self.blocks[block].data {
                let chunk = i % 8;
                block_data[chunk].read(
                    data_begin - data_begin_as_chunk,
                    &mut buf[data_begin - pos..data_end - pos],
                )?
            } else {
                for i in buf[data_begin - pos..data_end - pos].iter_mut() {
                    *i = 0xFF;
                }
            }
        }

        Ok(())
    }
    fn write(&self, _pos: usize, _buf: &[u8]) -> Result<(), Error> {
        unimplemented!()
    }
    fn len(&self) -> usize {
        // -1 for the reserved block
        (self.blocks.len() - 1) * 0x1000
    }
    fn commit(&self) -> Result<(), Error> {
        unimplemented!()
    }
}

#[cfg(test)]
pub mod test {
    use super::*;
    use crate::memory_file::MemoryFile;
    use rand::distributions::Standard;
    use rand::prelude::*;

    #[test]
    fn fuzz_crc() {
        let mut rng = rand::thread_rng();
        for _ in 0..10 {
            let len = rng.gen_range(1, 100);
            let init: Vec<u8> = rng.sample_iter(&Standard).take(len).collect();
            let crc = crc16_ninty(&init).to_le_bytes().to_vec();
            let crc = Rc::new(MemoryFile::new(crc));
            let data = Rc::new(MemoryFile::new(init));
            let mut file =
                CrcFile::new(SimpleCrcStub::new(crc.clone()).unwrap(), data.clone()).unwrap();
            let mut buf = vec![0; len];
            file.read(0, &mut buf).unwrap();
            let plain = MemoryFile::new(buf);
            crate::random_access_file::fuzzer(
                &mut file,
                |file| file,
                |file| file.commit().unwrap(),
                || CrcFile::new(SimpleCrcStub::new(crc.clone()).unwrap(), data.clone()).unwrap(),
                &plain,
                len,
            );
        }
    }

    #[test]
    fn fuzz_crc_xor() {
        let mut rng = rand::thread_rng();
        for _ in 0..10 {
            let len = rng.gen_range(1, 100);
            let init: Vec<u8> = rng.sample_iter(&Standard).take(len).collect();
            let crc = crc16_ninty(&init).to_le_bytes();
            let crc = vec![crc[0] ^ crc[1]];
            let crc = Rc::new(MemoryFile::new(crc));
            let data = Rc::new(MemoryFile::new(init));
            let mut file =
                CrcFile::new(XorCrcStub::new(crc.clone()).unwrap(), data.clone()).unwrap();
            let mut buf = vec![0; len];
            file.read(0, &mut buf).unwrap();
            let plain = MemoryFile::new(buf);
            crate::random_access_file::fuzzer(
                &mut file,
                |file| file,
                |file| file.commit().unwrap(),
                || CrcFile::new(XorCrcStub::new(crc.clone()).unwrap(), data.clone()).unwrap(),
                &plain,
                len,
            );
        }
    }

    #[test]
    fn fuzz_mirrored() {
        let mut rng = rand::thread_rng();
        for _ in 0..10 {
            let len = rng.gen_range(1, 100);
            let init0: Vec<u8> = rng.sample_iter(&Standard).take(len).collect();
            let init1: Vec<u8> = init0.clone();
            let data0 = Rc::new(MemoryFile::new(init0));
            let data1 = Rc::new(MemoryFile::new(init1));
            let mut file = MirroredFile::new(data0.clone(), data1.clone()).unwrap();
            let mut buf = vec![0; len];
            file.read(0, &mut buf).unwrap();
            let plain = MemoryFile::new(buf);
            crate::random_access_file::fuzzer(
                &mut file,
                |file| file,
                |file| file.commit().unwrap(),
                || MirroredFile::new(data0.clone(), data1.clone()).unwrap(),
                &plain,
                len,
            );
        }
    }
}
