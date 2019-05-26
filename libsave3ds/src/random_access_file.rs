use crate::error::*;
use byte_struct::*;
use std::borrow::Borrow;

pub trait RandomAccessFile {
    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error>;
    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error>;
    fn len(&self) -> usize;
    fn commit(&self) -> Result<(), Error>;
}

pub fn read_struct<T: ByteStruct>(f: &RandomAccessFile, pos: usize) -> Result<T, Error> {
    let mut buf = vec![0; T::BYTE_LEN]; // array somehow broken with the associated item as size
    f.borrow().read(pos, &mut buf)?;
    Ok(T::read_bytes(&buf))
}

pub fn write_struct<T: ByteStruct>(f: &RandomAccessFile, pos: usize, data: T) -> Result<(), Error> {
    let mut buf = vec![0; T::BYTE_LEN]; // array somehow broken with the associated item as size
    data.write_bytes(&mut buf);
    f.borrow().write(pos, &buf)?;
    Ok(())
}

#[cfg(test)]
pub fn fuzzer<
    Subject,
    SubjectFile: RandomAccessFile,
    SubjectAccessor: Fn(&Subject) -> &SubjectFile,
    SubjectCommitor: Fn(&Subject) -> (),
    SubjectReloader: Fn() -> Subject,
    Control: RandomAccessFile,
>(
    subject: &mut Subject,
    accessor: SubjectAccessor,
    commitor: SubjectCommitor,
    reloader: SubjectReloader,
    control: &Control,
    len: usize,
) {
    use rand::distributions::Standard;
    use rand::prelude::*;

    let mut rng = rand::thread_rng();
    for _ in 0..1000 {
        let operation = rng.gen_range(1, 10);
        if operation == 1 {
            commitor(subject);
            *subject = reloader();
        } else if operation < 4 {
            commitor(subject);
        } else {
            let pos = rng.gen_range(0, len);
            let data_len = rng.gen_range(1, len - pos + 1);
            if operation < 7 {
                let mut a = vec![0; data_len];
                let mut b = vec![0; data_len];
                accessor(subject).read(pos, &mut a).unwrap();
                control.read(pos, &mut b).unwrap();
                assert_eq!(a, b);
            } else {
                let a: Vec<u8> = rng.sample_iter(&Standard).take(data_len).collect();
                accessor(subject).write(pos, &a).unwrap();
                control.write(pos, &a).unwrap();
            }
        }
    }
}
