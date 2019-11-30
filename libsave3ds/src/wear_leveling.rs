use crate::byte_struct_common::*;
use crate::error::*;
use crate::memory_file::*;
use crate::misc::*;
use crate::random_access_file::*;
use crate::sub_file::SubFile;
use std::cell::*;
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
    fn new(crc_stub: C, data: Rc<F>, initialized: bool) -> Result<CrcFile<C, F>, Error> {
        let len = data.len();
        let mut buf = vec![0; len];
        data.read(0, &mut buf)?;
        if initialized && !crc_stub.verify(crc16_ninty(&buf))? {
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
    physical_block: u8,
    allocate_count: u8,
    initialized: bool,
    dirty: bool,
    crc_ticket: Option<Rc<MemoryFile>>,
    data: Vec<Box<dyn RandomAccessFile>>,
}

pub struct WearLeveling {
    block_map: Rc<CrcFile<SimpleCrcStub<SubFile>, SubFile>>,
    journal_list: Rc<SubFile>,
    blocks: RefCell<Vec<WearLevelingBlock>>,
    large_save: bool,
}

impl WearLeveling {
    pub fn format(parent: Rc<dyn RandomAccessFile>) -> Result<(), Error> {
        let len = parent.len();
        if len != 0x20_000 && len != 0x80_000 && len != 0x100_000 {
            return Err(Error::SizeMismatch);
        }
        let large_save = len == 0x100_000;

        let virtual_block_count = len / 0x1000 - 1;

        let block_map_len = if large_save {
            0x3FE
        } else {
            8 + virtual_block_count * 10
        };

        let block_map = Rc::new(SubFile::new(parent.clone(), 0, block_map_len)?);
        let block_map_crc = Rc::new(SubFile::new(parent.clone(), block_map_len, 2)?);
        let block_map = Rc::new(CrcFile::new(
            SimpleCrcStub::new(block_map_crc)?,
            block_map,
            false,
        )?);

        block_map.write(0, &[0; 8])?;

        let item_len = if large_save { 2 } else { 10 };
        for i in 0..virtual_block_count {
            if large_save {
                block_map.write(8 + i * item_len, &[0])?;
                block_map.write(8 + i * item_len + 1, &[(i + 1) as u8])?;
            } else {
                block_map.write(8 + i * item_len, &[(i + 1) as u8])?;
                block_map.write(8 + i * item_len + 1, &[0])?;
                block_map.write(8 + i * item_len + 2, &[0; 8])?;
            }
        }
        for pos in 8 + virtual_block_count * item_len..block_map_len {
            block_map.write(pos, &[0])?;
        }

        block_map.commit()?;

        let journal_start = block_map_len + 2;
        let journal_list = Rc::new(SubFile::new(
            parent.clone(),
            journal_start,
            0x1000 - journal_start,
        )?);

        for pos in 0..journal_list.len() {
            journal_list.write(pos, &[0xFF])?;
        }

        Ok(())
    }

    pub fn new(parent: Rc<dyn RandomAccessFile>) -> Result<WearLeveling, Error> {
        let len = parent.len();
        if len != 0x20_000 && len != 0x80_000 && len != 0x100_000 {
            return Err(Error::SizeMismatch);
        }
        let large_save = len == 0x100_000;
        let physical_block_count = len / 0x1000;
        let virtual_block_count = physical_block_count - 1;

        let block_map_len = if large_save {
            0x3FE
        } else {
            8 + virtual_block_count * 10
        };

        let block_map = Rc::new(SubFile::new(parent.clone(), 0, block_map_len)?);
        let block_map_crc = Rc::new(SubFile::new(parent.clone(), block_map_len, 2)?);
        let block_map = Rc::new(CrcFile::new(
            SimpleCrcStub::new(block_map_crc)?,
            block_map,
            true,
        )?);

        struct Block {
            physical_block: u8,
            allocate_count: u8,
            initialized: bool,
            crc_ticket: Option<MemoryFile>,
        };

        let mut blocks = vec![];
        let item_len = if large_save { 2 } else { 10 };
        for i in 0..virtual_block_count {
            let offset = i * item_len + 8;
            let mut buf = [0; 2];
            block_map.read(offset, &mut buf)?;
            let initialized = buf[0] & 0x80 != 0;
            let crc_ticket = if large_save {
                None
            } else if initialized {
                Some(MemoryFile::from_file(&SubFile::new(
                    block_map.clone(),
                    offset + 2,
                    8,
                )?)?)
            } else {
                Some(MemoryFile::new(vec![0; 8]))
            };
            let physical_block = if large_save { buf[1] } else { buf[0] & 0x7F };
            let allocate_count = if large_save { buf[0] & 0x7F } else { buf[1] };
            blocks.push(Block {
                physical_block,
                allocate_count,
                initialized,
                crc_ticket,
            });
        }

        let mut physical_block_set: HashSet<_> = (1..physical_block_count).collect();
        for block in blocks.iter() {
            if !physical_block_set.remove(&(block.physical_block as usize)) {
                return Err(Error::InvalidValue);
            }
        }

        let journal_start = block_map_len + 2;
        let journal_list = Rc::new(SubFile::new(
            parent.clone(),
            journal_start,
            0x1000 - journal_start,
        )?);

        for offset in (0..journal_list.len()).step_by(0x20) {
            let journal0 = Rc::new(SubFile::new(journal_list.clone(), offset, 14)?);
            let journal1 = Rc::new(SubFile::new(journal_list.clone(), offset + 14, 14)?);
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

            if blocks[virtual_block_prev].initialized {
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
            blocks[virtual_block_prev].initialized = false;
            if !large_save {
                blocks[virtual_block_prev].crc_ticket = Some(MemoryFile::new(vec![0; 8]));
            }
            blocks[virtual_block].allocate_count = allocate_count;
            blocks[virtual_block].physical_block = physical_block;
            blocks[virtual_block].initialized = true;
            if !large_save {
                blocks[virtual_block].crc_ticket = Some(MemoryFile::from_file(
                    &(SubFile::new(Rc::new(journal), 6, 8)?),
                )?);
            }
        }

        if blocks.last().unwrap().initialized {
            return Err(Error::InvalidValue);
        }

        let mut final_blocks = vec![];
        for block in blocks {
            let mut data_list: Vec<Box<dyn RandomAccessFile>> = vec![];
            let crc_ticket = block.crc_ticket.map(Rc::new);
            for i in 0..8 {
                let offset = i * 0x200 + block.physical_block as usize * 0x1000;
                let data = SubFile::new(parent.clone(), offset, 0x200)?;
                let data: Box<dyn RandomAccessFile> = if let Some(crc_ticket) = crc_ticket.clone() {
                    let crc = Rc::new(SubFile::new(crc_ticket.clone(), i, 1)?);
                    Box::new(CrcFile::new(
                        XorCrcStub::new(crc)?,
                        Rc::new(data),
                        block.initialized,
                    )?)
                } else {
                    Box::new(data)
                };
                data_list.push(data);
            }
            final_blocks.push(WearLevelingBlock {
                physical_block: block.physical_block,
                allocate_count: block.allocate_count,
                initialized: block.initialized,
                dirty: false,
                crc_ticket,
                data: data_list,
            });
        }

        Ok(WearLeveling {
            block_map,
            journal_list,
            blocks: RefCell::new(final_blocks),
            large_save,
        })
    }
}

const CHUNK_INIT: [u8; 0x200] = [0xFF; 0x200];

impl RandomAccessFile for WearLeveling {
    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error> {
        let end = pos + buf.len();

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

            let block = &self.blocks.borrow()[i / 8];
            if block.initialized {
                let chunk = i % 8;
                block.data[chunk].read(
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
    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error> {
        // TODO: implement proper reallocating

        let end = pos + buf.len();

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

            let block = &mut self.blocks.borrow_mut()[i / 8];
            if !block.initialized {
                block.initialized = true;
                if block.allocate_count == 0 {
                    block.allocate_count = 1;
                }
                for chunk in block.data.iter() {
                    chunk.write(0, &CHUNK_INIT)?;
                }
            }

            block.dirty = true;

            let chunk = i % 8;
            block.data[chunk].write(
                data_begin - data_begin_as_chunk,
                &buf[data_begin - pos..data_end - pos],
            )?
        }

        Ok(())
    }
    fn len(&self) -> usize {
        // -1 for the reserved block
        (self.blocks.borrow().len() - 1) * 0x1000
    }
    fn commit(&self) -> Result<(), Error> {
        // TODO: implement proper reallocating and journal recording.
        // we now simply squash the journal.
        let item_len = if self.large_save { 2 } else { 10 };
        for (i, block) in self.blocks.borrow_mut().iter_mut().enumerate() {
            if block.initialized && block.dirty {
                for data in block.data.iter() {
                    data.commit()?;
                }
                block.dirty = false;
            }

            let buf = if self.large_save {
                [
                    block.allocate_count + ((block.initialized as u8) << 7),
                    block.physical_block,
                ]
            } else {
                [
                    block.physical_block + ((block.initialized as u8) << 7),
                    block.allocate_count,
                ]
            };

            self.block_map.write(8 + item_len * i, &buf)?;

            if let Some(block_crc_ticket) = &block.crc_ticket {
                let mut crc_ticket = [0; 8];
                block_crc_ticket.read(0, &mut crc_ticket)?;
                self.block_map.write(8 + item_len * i + 2, &crc_ticket)?;
            }
        }

        self.block_map.commit()?;

        for offset in 0..self.journal_list.len() {
            self.journal_list.write(offset, &[0xFF])?;
        }

        Ok(())
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
            let file =
                CrcFile::new(SimpleCrcStub::new(crc.clone()).unwrap(), data.clone(), true).unwrap();
            let mut buf = vec![0; len];
            file.read(0, &mut buf).unwrap();
            let plain = MemoryFile::new(buf);
            crate::random_access_file::fuzzer(
                file,
                |file| file,
                |file| file.commit().unwrap(),
                || {
                    CrcFile::new(SimpleCrcStub::new(crc.clone()).unwrap(), data.clone(), true)
                        .unwrap()
                },
                plain,
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
            let file =
                CrcFile::new(XorCrcStub::new(crc.clone()).unwrap(), data.clone(), true).unwrap();
            let mut buf = vec![0; len];
            file.read(0, &mut buf).unwrap();
            let plain = MemoryFile::new(buf);
            crate::random_access_file::fuzzer(
                file,
                |file| file,
                |file| file.commit().unwrap(),
                || CrcFile::new(XorCrcStub::new(crc.clone()).unwrap(), data.clone(), true).unwrap(),
                plain,
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
            let file = MirroredFile::new(data0.clone(), data1.clone()).unwrap();
            let mut buf = vec![0; len];
            file.read(0, &mut buf).unwrap();
            let plain = MemoryFile::new(buf);
            crate::random_access_file::fuzzer(
                file,
                |file| file,
                |file| file.commit().unwrap(),
                || MirroredFile::new(data0.clone(), data1.clone()).unwrap(),
                plain,
            );
        }
    }

    #[test]
    fn fuzz_wear_leveling_small() {
        let mut rng = rand::thread_rng();
        for i in 0..10 {
            let len = if rng.gen() { 0x20_000 } else { 0x80_000 };
            let virtual_block_count = len / 0x1000 - 1;
            let init = Rc::new(MemoryFile::new(vec![0xFF; len]));
            let plain = MemoryFile::new(vec![0xFF; len - 0x2000]);

            if i % 2 == 0 {
                use rand::seq::SliceRandom;
                let mut blocks: Vec<_> = (1..(virtual_block_count + 1) as u8).collect();
                blocks[..].shuffle(&mut rng);

                let block_map =
                    Rc::new(SubFile::new(init.clone(), 0, 8 + virtual_block_count * 10).unwrap());
                let block_map_crc =
                    Rc::new(SubFile::new(init.clone(), 8 + virtual_block_count * 10, 2).unwrap());
                let block_map = Rc::new(
                    CrcFile::new(SimpleCrcStub::new(block_map_crc).unwrap(), block_map, false)
                        .unwrap(),
                );

                for (i, block) in blocks.into_iter().enumerate() {
                    block_map.write(8 + i * 10, &[block]).unwrap();
                    block_map.write(8 + i * 10 + 1, &[rng.gen()]).unwrap();
                    block_map.write(8 + i * 10 + 2, &[0; 8]).unwrap();
                }
                block_map.commit().unwrap();
                std::mem::drop(block_map);
            // TODO: random journal
            } else {
                WearLeveling::format(init.clone()).unwrap();
            }

            let file = WearLeveling::new(init.clone()).unwrap();
            crate::random_access_file::fuzzer(
                file,
                |file| file,
                |file| file.commit().unwrap(),
                || WearLeveling::new(init.clone()).unwrap(),
                plain,
            );
        }
    }

    #[test]
    fn fuzz_wear_leveling_large() {
        let mut rng = rand::thread_rng();
        for i in 0..10 {
            let len = 0x100_000;
            let init = Rc::new(MemoryFile::new(vec![0xFF; len]));
            let plain = MemoryFile::new(vec![0xFF; len - 0x2000]);

            if i % 2 == 0 {
                use rand::seq::SliceRandom;
                let mut blocks: Vec<_> = (1..=255).collect();
                blocks[..].shuffle(&mut rng);

                let block_map = Rc::new(SubFile::new(init.clone(), 0, 0x3FE).unwrap());
                let block_map_crc = Rc::new(SubFile::new(init.clone(), 0x3FE, 2).unwrap());
                let block_map = Rc::new(
                    CrcFile::new(SimpleCrcStub::new(block_map_crc).unwrap(), block_map, false)
                        .unwrap(),
                );

                for (i, block) in blocks.into_iter().enumerate() {
                    block_map
                        .write(8 + i * 2, &[rng.gen::<u8>() & 0x7F])
                        .unwrap();
                    block_map.write(8 + i * 2 + 1, &[block]).unwrap();
                }
                block_map.commit().unwrap();

                std::mem::drop(block_map);
            // TODO: random journal
            } else {
                WearLeveling::format(init.clone()).unwrap();
            }

            let file = WearLeveling::new(init.clone()).unwrap();
            crate::random_access_file::fuzzer(
                file,
                |file| file,
                |file| file.commit().unwrap(),
                || WearLeveling::new(init.clone()).unwrap(),
                plain,
            );
        }
    }
}
