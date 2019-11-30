use crate::error::*;
use crate::misc::*;
use crate::random_access_file::*;
use std::cell::RefCell;
use std::rc::Rc;

pub struct DpfsLevel {
    selector: Rc<dyn RandomAccessFile>,
    pair: [Rc<dyn RandomAccessFile>; 2],
    block_len: usize,
    len: usize,
    dirty: RefCell<Vec<u32>>,
}

impl DpfsLevel {
    pub fn new(
        selector: Rc<dyn RandomAccessFile>,
        pair: [Rc<dyn RandomAccessFile>; 2],
        block_len: usize,
    ) -> Result<DpfsLevel, Error> {
        let len = pair[0].len();
        if pair[1].len() != len {
            return make_error(Error::SizeMismatch);
        }
        let block_count = divide_up(len, block_len);
        let chunk_count = divide_up(block_count, 32);
        if chunk_count * 4 > selector.len() {
            return make_error(Error::SizeMismatch);
        }

        Ok(DpfsLevel {
            selector,
            pair,
            block_len,
            len,
            dirty: RefCell::new(vec![0; chunk_count]),
        })
    }
}

impl RandomAccessFile for DpfsLevel {
    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error> {
        let end = pos + buf.len();
        if end > self.len() {
            return make_error(Error::OutOfBound);
        }

        // block index range the operation covers
        let begin_block = pos / self.block_len;
        let end_block = divide_up(end, self.block_len);

        // chunk index range the operation covers
        let begin_chunk = begin_block / 32;
        let end_chunk = divide_up(end_block, 32);

        // read all related selectors
        let mut selector = vec![0; (end_chunk - begin_chunk) * 4];
        self.selector.read(begin_chunk * 4, &mut selector)?;

        for chunk_i in begin_chunk..end_chunk {
            // we are going to read from the active partition if the block is clean;
            // otherwise we read from the inactive partition
            let dirty = self.dirty.borrow()[chunk_i];
            let raw = &selector[(chunk_i - begin_chunk) * 4..(chunk_i + 1 - begin_chunk) * 4];
            let select = dirty ^ u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);

            // block index range we operate on within this chunk
            let block_i_begin = std::cmp::max(chunk_i * 32, begin_block);
            let block_i_end = std::cmp::min((chunk_i + 1) * 32, end_block);

            for block_i in block_i_begin..block_i_end {
                // the partition we are going to read from
                let select_bit = (select >> (31 - (block_i - chunk_i * 32))) & 1;

                // data range we operate on within this block
                let data_begin = std::cmp::max(block_i * self.block_len, pos);
                let data_end = std::cmp::min((block_i + 1) * self.block_len, end);

                // read the data
                self.pair[select_bit as usize]
                    .read(data_begin, &mut buf[data_begin - pos..data_end - pos])?;
            }
        }

        Ok(())
    }
    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error> {
        let end = pos + buf.len();
        if end > self.len() {
            return make_error(Error::OutOfBound);
        }

        // block index range the operation covers
        let begin_block = pos / self.block_len;
        let end_block = divide_up(end, self.block_len);

        // chunk index range the operation covers
        let begin_chunk = begin_block / 32;
        let end_chunk = divide_up(end_block, 32);

        // read all related selectors
        let mut selector = vec![0; (end_chunk - begin_chunk) * 4];
        self.selector.read(begin_chunk * 4, &mut selector)?;

        for chunk_i in begin_chunk..end_chunk {
            let dirty = &mut self.dirty.borrow_mut()[chunk_i];

            // we always write to the inactive partition
            let raw = &selector[(chunk_i - begin_chunk) * 4..(chunk_i + 1 - begin_chunk) * 4];
            let select = !u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);

            // block index range we operate on within this chunk
            let block_i_begin = std::cmp::max(chunk_i * 32, begin_block);
            let block_i_end = std::cmp::min((chunk_i + 1) * 32, end_block);

            for block_i in block_i_begin..block_i_end {
                // the partition (inactive partition) we are going to write to
                let shift = 31 - (block_i - chunk_i * 32);
                let select_bit = (select >> shift) & 1;

                // data range this block covers
                let data_begin_as_block = block_i * self.block_len;
                let data_end_as_block = std::cmp::min((block_i + 1) * self.block_len, self.len);

                // data range we operate on within this block
                let data_begin = std::cmp::max(data_begin_as_block, pos);
                let data_end = std::cmp::min(data_end_as_block, end);

                // write the data
                self.pair[select_bit as usize]
                    .write(data_begin, &buf[data_begin - pos..data_end - pos])?;

                // if the block was clean, and we have just written an incomplete block,
                // we need to transfer the margin data from the active partition to the inactive partition.
                let keep_bit = (*dirty >> shift) & 1;
                if keep_bit == 0 {
                    let other = 1 - select_bit;
                    // left margin
                    if data_begin > data_begin_as_block {
                        let mut block_buf = vec![0; data_begin - data_begin_as_block];
                        self.pair[other as usize].read(data_begin_as_block, &mut block_buf)?;
                        self.pair[select_bit as usize].write(data_begin_as_block, &block_buf)?;
                    }

                    // right margin
                    if data_end < data_end_as_block {
                        let mut block_buf = vec![0; data_end_as_block - data_end];
                        self.pair[other as usize].read(data_end, &mut block_buf)?;
                        self.pair[select_bit as usize].write(data_end, &block_buf)?;
                    }
                }

                // set the dirty bit
                *dirty |= 1 << shift;
            }
        }

        Ok(())
    }
    fn len(&self) -> usize {
        self.len
    }
    fn commit(&self) -> Result<(), Error> {
        // Flip selector bits for all dirty blocks
        let mut dirty = self.dirty.borrow_mut();
        for (i, word) in dirty.iter_mut().enumerate() {
            if *word != 0 {
                let mut bytes = [0; 4];
                self.selector.read(i * 4, &mut bytes)?;
                let old_word = u32::from_le_bytes(bytes);
                let bytes = (old_word ^ *word).to_le_bytes();
                self.selector.write(i * 4, &bytes)?;
                *word = 0;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::dpfs_level::DpfsLevel;
    use crate::memory_file::MemoryFile;
    use crate::misc::*;
    use crate::random_access_file::*;
    use std::rc::Rc;

    #[test] #[rustfmt::skip]
    fn test() {
        let selector = Rc::new(MemoryFile::new(vec![0xF0, 0x0F, 0xFF, 0x00, 0xA0, 0xAA, 0x55, 0x55]));
        let pair: [Rc<dyn RandomAccessFile>; 2] = [Rc::new(MemoryFile::new(vec![
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,

            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,

            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,

            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF
        ])), Rc::new(MemoryFile::new(vec![
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,

            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,

            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,

            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00,
        ]))];

        let file = DpfsLevel::new(selector.clone(), pair.clone(), 2).unwrap();
        assert_eq!(file.len(), 128 - 7);
        let mut buf1 = [0; 16];
        file.read(65, &mut buf1).unwrap();
        assert_eq!(buf1, [
            0xFF, 0x02, 0x03, 0xFF, 0xFF, 0x06, 0x07, 0xFF,
            0xFF, 0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0xFF]
        );

        let buf2 = [0x11, 0x22, 0x33, 0x44, 0x55];
        file.write(100, &buf2).unwrap();
        let mut buf3 = [0; 7];
        file.read(99, &mut buf3).unwrap();
        assert_eq!(buf3, [0xFF, 0x11, 0x22, 0x33, 0x44, 0x55, 0x00]);
        file.commit().unwrap();
        file.read(99, &mut buf3).unwrap();
        assert_eq!(buf3, [0xFF, 0x11, 0x22, 0x33, 0x44, 0x55, 0x00]);
        let file = DpfsLevel::new(selector, pair, 2).unwrap();
        file.read(99, &mut buf3).unwrap();
        assert_eq!(buf3, [0xFF, 0x11, 0x22, 0x33, 0x44, 0x55, 0x00]);
    }

    #[test]
    fn fuzz() {
        use rand::distributions::Standard;
        use rand::prelude::*;

        let mut rng = rand::thread_rng();
        for _ in 0..10 {
            let len = rng.gen_range(1, 10_000);
            let block_len = rng.gen_range(1, 100);
            let block_count = divide_up(len, block_len);
            let chunk_count = divide_up(block_count, 32);
            let selector_len = chunk_count * 4;
            let selector = Rc::new(MemoryFile::new(
                rng.sample_iter(&Standard).take(selector_len).collect(),
            ));
            let pair: [Rc<dyn RandomAccessFile>; 2] = [
                Rc::new(MemoryFile::new(
                    rng.sample_iter(&Standard).take(len).collect(),
                )),
                Rc::new(MemoryFile::new(
                    rng.sample_iter(&Standard).take(len).collect(),
                )),
            ];
            let init: Vec<u8> = rng.sample_iter(&Standard).take(len).collect();
            let dpfs_level = DpfsLevel::new(selector.clone(), pair.clone(), block_len).unwrap();
            dpfs_level.write(0, &init).unwrap();
            let plain = MemoryFile::new(init);

            crate::random_access_file::fuzzer(
                dpfs_level,
                |file| file,
                |file| file.commit().unwrap(),
                || DpfsLevel::new(selector.clone(), pair.clone(), block_len).unwrap(),
                plain,
            );
        }
    }
}
