use crate::error::*;
use crate::random_access_file::*;
use aes::*;
use cmac::crypto_mac::generic_array::*;
use cmac::*;
use sha2::*;
use std::rc::Rc;

pub trait Signer {
    fn hash(&self, data: Vec<u8>) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.input(&self.block(data));
        hasher.result().into_iter().collect()
    }
    fn block(&self, data: Vec<u8>) -> Vec<u8>;
}

pub struct SignedFile {
    signature: Rc<RandomAccessFile>,
    data: Rc<RandomAccessFile>,
    block_provider: Box<Signer>,
    key: [u8; 16],
    len: usize,
}

impl SignedFile {
    pub fn new(
        signature: Rc<RandomAccessFile>,
        data: Rc<RandomAccessFile>,
        block_provider: Box<Signer>,
        key: [u8; 16],
    ) -> Result<SignedFile, Error> {
        if signature.len() != 16 {
            return make_error(Error::SizeMismatch);
        }
        let len = data.len();
        let file = SignedFile {
            signature,
            data,
            block_provider,
            key,
            len,
        };

        let mut signature = [0; 16];
        file.signature.read(0, &mut signature)?;
        if signature != file.calculate_signature()? {
            return make_error(Error::SignatureMismatch);
        }

        Ok(file)
    }

    fn calculate_signature(&self) -> Result<[u8; 16], Error> {
        let mut data = vec![0; self.len];
        self.data.read(0, &mut data)?;
        let hash = self.block_provider.hash(data);
        let mut cmac: Cmac<Aes128> = Cmac::new(GenericArray::from_slice(&self.key));
        cmac.input(&hash);
        let mut result = [0; 16];
        result.copy_from_slice(cmac.result().code().as_slice());
        Ok(result)
    }
}

impl RandomAccessFile for SignedFile {
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
        self.signature.write(0, &self.calculate_signature()?)
    }
}

#[cfg(test)]
mod test {
    use crate::memory_file::MemoryFile;
    use crate::random_access_file::*;
    use crate::signed_file::*;
    use std::rc::Rc;

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
    fn fuzz() {
        use rand::distributions::Standard;
        use rand::prelude::*;

        let mut rng = rand::thread_rng();
        for _ in 0..10 {
            let len = rng.gen_range(1, 100);
            let init: Vec<u8> = rng.sample_iter(&Standard).take(len).collect();

            let signer = Box::new(SimpleSigner { salt: rng.gen() });
            let key: [u8; 16] = rng.gen();

            let hash = signer.hash(init.clone());
            let mut cmac: Cmac<Aes128> = Cmac::new(GenericArray::from_slice(&key));
            cmac.input(&hash);
            let mut cmac_result = vec![0; 16];
            cmac_result.copy_from_slice(cmac.result().code().as_slice());

            let data = Rc::new(MemoryFile::new(init));
            let signature = Rc::new(MemoryFile::new(cmac_result));

            let mut file =
                SignedFile::new(signature.clone(), data.clone(), signer.clone(), key).unwrap();
            let mut buf = vec![0; len];
            file.read(0, &mut buf).unwrap();
            let plain = MemoryFile::new(buf);

            crate::random_access_file::fuzzer(
                &mut file,
                |file| file,
                |file| file.commit().unwrap(),
                || SignedFile::new(signature.clone(), data.clone(), signer.clone(), key).unwrap(),
                &plain,
                len,
            );
        }
    }
}
