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
