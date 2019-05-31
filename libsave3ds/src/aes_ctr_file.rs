use crate::error::*;
use crate::misc::*;
use crate::random_access_file::*;
use aes::block_cipher_trait::generic_array::GenericArray;
use aes::block_cipher_trait::*;
use aes::*;
use lru::LruCache;
use std::cell::RefCell;
use std::rc::Rc;

pub struct AesCtrFile {
    data: Rc<RandomAccessFile>,
    aes128: Aes128,
    ctr: [u8; 16],
    len: usize,
    cache: RefCell<LruCache<usize, [u8; 16]>>,
}

fn seek_ctr(ctr: &mut [u8; 16], mut block_index: usize) {
    for i in (8..16).rev() {
        block_index += ctr[i] as usize;
        ctr[i] = (block_index & 0xFF) as u8;
        block_index >>= 8;
    }
}

impl AesCtrFile {
    pub fn new(data: Rc<RandomAccessFile>, key: [u8; 16], ctr: [u8; 16]) -> AesCtrFile {
        let len = data.len();
        let aes128 = Aes128::new(GenericArray::from_slice(&key));
        AesCtrFile {
            data,
            aes128,
            ctr,
            len,
            cache: RefCell::new(LruCache::new(16)),
        }
    }

    fn get_pad(&self, block_index: usize) -> [u8; 16] {
        let mut cache = self.cache.borrow_mut();
        if let Some(cached) = cache.get(&block_index) {
            *cached
        } else {
            let mut ctr = self.ctr;
            seek_ctr(&mut ctr, block_index);
            let block_buf = GenericArray::from_mut_slice(&mut ctr);
            self.aes128.encrypt_block(block_buf);
            cache.put(block_index, ctr);
            ctr
        }
    }
}
impl RandomAccessFile for AesCtrFile {
    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error> {
        let end = pos + buf.len();
        if end > self.len() {
            return make_error(Error::OutOfBound);
        }
        self.data.read(pos, buf)?;

        // block index range the operation covers
        let begin_block = pos / 16;
        let end_block = divide_up(end, 16);

        let mut ctr = self.ctr;
        seek_ctr(&mut ctr, begin_block);
        for i in begin_block..end_block {
            let pad = self.get_pad(i);

            let data_begin = std::cmp::max(i * 16, pos);
            let data_end = std::cmp::min((i + 1) * 16, end);

            for p in data_begin..data_end {
                buf[p - pos] ^= pad[p - i * 16];
            }

            seek_ctr(&mut ctr, 1);
        }

        Ok(())
    }
    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error> {
        let end = pos + buf.len();
        if end > self.len() {
            return make_error(Error::OutOfBound);
        }

        // block index range the operation covers
        let begin_block = pos / 16;
        let end_block = divide_up(end, 16);

        let mut ctr = self.ctr;
        seek_ctr(&mut ctr, begin_block);
        for i in begin_block..end_block {
            let mut pad = self.get_pad(i);

            let data_begin = std::cmp::max(i * 16, pos);
            let data_end = std::cmp::min((i + 1) * 16, end);

            for p in data_begin..data_end {
                pad[p - i * 16] ^= buf[p - pos];
            }

            self.data
                .write(data_begin, &pad[data_begin - i * 16..data_end - i * 16])?;

            seek_ctr(&mut ctr, 1);
        }

        Ok(())
    }
    fn len(&self) -> usize {
        self.len
    }
    fn commit(&self) -> Result<(), Error> {
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::aes_ctr_file::AesCtrFile;
    use crate::memory_file::MemoryFile;
    use crate::random_access_file::*;
    use std::rc::Rc;
    #[test]
    fn fuzz() {
        use rand::distributions::Standard;
        use rand::prelude::*;

        let mut rng = rand::thread_rng();
        for _ in 0..10 {
            let len = rng.gen_range(1, 1000);
            let data = Rc::new(MemoryFile::new(
                rng.sample_iter(&Standard).take(len).collect(),
            ));
            let key: [u8; 16] = rng.gen();
            let ctr: [u8; 16] = rng.gen();
            let mut aes_ctr_file = AesCtrFile::new(data.clone(), key, ctr);
            let mut init: Vec<u8> = vec![0; len];
            aes_ctr_file.read(0, &mut init).unwrap();
            let plain = MemoryFile::new(init);

            crate::random_access_file::fuzzer(
                &mut aes_ctr_file,
                |aes_ctr_file| aes_ctr_file,
                |aes_ctr_file| aes_ctr_file.commit().unwrap(),
                || AesCtrFile::new(data.clone(), key, ctr),
                &plain,
                len,
            );
        }
    }
}
