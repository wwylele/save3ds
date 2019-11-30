use crate::error::*;
use byte_struct::*;
use std::borrow::Borrow;

/// Interface to a file that supports random access.
///
/// A `RandomAccessFile` acts similar to a fixed-size array `[u8; len()]`, except that
///  - It is not necessarily stored as a simple array in memory.
///    It can be a physical file on the harddrive, or an encrypted array.
///  - Every read and write operation can potentially, though rarely, results in an error.
///  - Each byte allows to have an additional "uninitialized" state.
///
/// Many implementations of `RandomAccessFile` act as a "layer": they transforms data
/// between the interface level and some other `RandomAccessFile`s as the underlying storage.
pub trait RandomAccessFile {
    /// Reads bytes at position `pos` into `buf`. The lenth is determined by `buf.len()`.
    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error>;

    /// Writes bytes to position `pos` from `buf`. The lenth is determined by `buf.len()`.
    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error>;

    /// Returns the length of this file.
    fn len(&self) -> usize;

    /// Flushes all changes made to the file,
    /// so that when the same file is opened after dropping this one,
    /// all data can be fully recovered.
    ///
    /// For a `RandomAccessFile` represents a phisycal file,
    /// this is equivalent to flushing the physical file to the disk.
    /// For a `RandomAccessFile` that acts as a layer, this flushes data
    /// to the underlying `RandomAccessFile`. Note that this doesn't recursively
    /// call commit on the underlying file.
    fn commit(&self) -> Result<(), Error>;
}

/// Helper for reading a `ByteStruct` from a `RandomAccessFile`.
pub fn read_struct<T: ByteStruct>(f: &dyn RandomAccessFile, pos: usize) -> Result<T, Error> {
    let mut buf = vec![0; T::BYTE_LEN]; // array somehow broken with the associated item as size
    f.borrow().read(pos, &mut buf)?;
    Ok(T::read_bytes(&buf))
}

/// Helper for writing a `ByteStruct` to a `RandomAccessFile`.
pub fn write_struct<T: ByteStruct>(
    f: &dyn RandomAccessFile,
    pos: usize,
    data: T,
) -> Result<(), Error> {
    let mut buf = vec![0; T::BYTE_LEN]; // array somehow broken with the associated item as size
    data.write_bytes(&mut buf);
    f.borrow().write(pos, &buf)?;
    Ok(())
}

/// Driver for fuzz test an implementation for `RandomAccessFile`.
///
/// - `subject`: the object that contains the `RandomAccessFile` implementation to test.
/// - `accessor`: method to access the `RandomAccessFile` implementation from the subject.
/// - `commitor`: method to commit the file from the subject.
///   We don't call RandomAccessFile::commit directly because
///   the subject might have additional stuff to commit.
/// - `reloader`: method to create a new subject of the same type for testing commit + drop + open.
/// - `control`: a different `RandomAccessFile` implementation for data verification.
#[cfg(test)]
pub fn fuzzer<Subject, SubjectFile: RandomAccessFile, Control: RandomAccessFile>(
    mut subject: Subject,
    accessor: impl Fn(&Subject) -> &SubjectFile,
    commitor: impl Fn(&Subject) -> (),
    reloader: impl Fn() -> Subject,
    control: Control,
) {
    use rand::distributions::Standard;
    use rand::prelude::*;

    let len = accessor(&subject).len();
    let mut rng = rand::thread_rng();
    for _ in 0..1000 {
        let operation = rng.gen_range(1, 10);
        if operation == 1 {
            commitor(&subject);
            subject = reloader();
        } else if operation < 4 {
            commitor(&subject);
        } else {
            let pos = rng.gen_range(0, len);
            let data_len = rng.gen_range(1, len - pos + 1);
            if operation < 7 {
                let mut a = vec![0; data_len];
                let mut b = vec![0; data_len];
                accessor(&subject).read(pos, &mut a).unwrap();
                control.read(pos, &mut b).unwrap();
                assert_eq!(a, b);
            } else {
                let a: Vec<u8> = rng.sample_iter(&Standard).take(data_len).collect();
                accessor(&subject).write(pos, &a).unwrap();
                control.write(pos, &a).unwrap();
            }
        }
    }
}
