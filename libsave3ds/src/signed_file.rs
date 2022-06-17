use crate::error::*;
use crate::random_access_file::*;
use aes::*;
use cmac::*;
use log::*;
use sha2::*;
use std::rc::Rc;

/// Abstract interface for transforming the file data into a block ready for hash and CMAC.
pub trait Signer {
    fn hash(&self, data: Vec<u8>) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(&self.block(data));
        hasher.finalize().into_iter().collect()
    }
    fn block(&self, data: Vec<u8>) -> Vec<u8>;
}

/// Implements `RandomAccessFile` layer as a file with a AES-CMAC signature.
pub struct SignedFile {
    signature: Rc<dyn RandomAccessFile>,
    data: Rc<dyn RandomAccessFile>,
    signer: Box<dyn Signer>,
    key: [u8; 16], // AES-CMAC key
    len: usize,
}

impl SignedFile {
    pub fn new_unverified(
        signature: Rc<dyn RandomAccessFile>,
        data: Rc<dyn RandomAccessFile>,
        signer: Box<dyn Signer>,
        key: [u8; 16],
    ) -> Result<SignedFile, Error> {
        if signature.len() != 16 {
            return make_error(Error::SizeMismatch);
        }
        let len = data.len();
        let file = SignedFile {
            signature,
            data,
            signer,
            key,
            len,
        };
        Ok(file)
    }

    pub fn new(
        signature: Rc<dyn RandomAccessFile>,
        data: Rc<dyn RandomAccessFile>,
        signer: Box<dyn Signer>,
        key: [u8; 16],
    ) -> Result<SignedFile, Error> {
        if signature.len() != 16 {
            return make_error(Error::SizeMismatch);
        }
        let len = data.len();
        let file = SignedFile {
            signature,
            data,
            signer,
            key,
            len,
        };

        let mut signature = [0; 16];
        file.signature.read(0, &mut signature)?;
        if signature != file.calculate_signature()? {
            error!("Signature mismatch");
            return make_error(Error::SignatureMismatch);
        }

        Ok(file)
    }

    fn calculate_signature(&self) -> Result<[u8; 16], Error> {
        let mut data = vec![0; self.len];
        self.data.read(0, &mut data)?;
        let hash = self.signer.hash(data);
        let mut cmac: Cmac<Aes128> = Cmac::new((&self.key).into());
        cmac.update(&hash);
        let mut result = [0; 16];
        result.copy_from_slice(cmac.finalize().into_bytes().as_slice());
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
pub mod test {
    use crate::memory_file::MemoryFile;
    use crate::random_access_file::*;
    use crate::signed_file::*;
    use std::rc::Rc;

    #[derive(Clone)]
    pub struct SimpleSigner {
        salt: u8,
    }

    impl SimpleSigner {
        pub fn new() -> SimpleSigner {
            use rand::prelude::*;
            let mut rng = rand::thread_rng();
            SimpleSigner { salt: rng.gen() }
        }
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
            let len = rng.gen_range(1..100);
            let init: Vec<u8> = (&mut rng).sample_iter(&Standard).take(len).collect();

            let signer = Box::new(SimpleSigner::new());
            let key: [u8; 16] = rng.gen();

            let hash = signer.hash(init.clone());
            let mut cmac: Cmac<Aes128> = Cmac::new((&key).into());
            cmac.update(&hash);
            let mut cmac_result = vec![0; 16];
            cmac_result.copy_from_slice(cmac.finalize().into_bytes().as_slice());

            let data = Rc::new(MemoryFile::new(init));
            let signature = Rc::new(MemoryFile::new(cmac_result));

            let file =
                SignedFile::new(signature.clone(), data.clone(), signer.clone(), key).unwrap();
            let mut buf = vec![0; len];
            file.read(0, &mut buf).unwrap();
            let plain = MemoryFile::new(buf);

            crate::random_access_file::fuzzer(
                file,
                |file| file,
                |file| file.commit().unwrap(),
                || SignedFile::new(signature.clone(), data.clone(), signer.clone(), key).unwrap(),
                plain,
            );
        }
    }
}
