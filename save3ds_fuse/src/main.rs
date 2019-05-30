use getopts::Options;
use libsave3ds::db::*;
use libsave3ds::error::*;
use libsave3ds::ext_data::*;
use libsave3ds::file_system;
use libsave3ds::save_data::*;
use libsave3ds::Resource;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::Read;
use std::rc::Rc;

#[cfg(all(unix, feature = "unixfuse"))]
use {
    fuse::*,
    libc::{
        getegid, geteuid, EBADF, EEXIST, EIO, EISDIR, ENAMETOOLONG, ENOENT, ENOSPC, ENOSYS,
        ENOTDIR, ENOTEMPTY, EROFS,
    },
    time,
};

enum FileSystemOperation {
    Mount(bool),
    Extract,
    Import,
}

#[allow(unused)]
struct FileSystemFrontend<T: file_system::FileSystem> {
    save: Rc<T::CenterType>,
    read_only: bool,
    fh_map: HashMap<u64, T::FileType>,
    next_fh: u64,
    uid: u32,
    gid: u32,
}

impl<T: file_system::FileSystem> FileSystemFrontend<T>
where
    T::NameType: NameConvert + Clone,
{
    fn new(save: Rc<T::CenterType>) -> FileSystemFrontend<T> {
        FileSystemFrontend::<T> {
            save,
            fh_map: HashMap::new(),
            next_fh: 1,
            read_only: true,
            uid: 0,
            gid: 0,
        }
    }

    fn extract_impl(
        &self,
        dir: T::DirType,
        path: &std::path::Path,
        indent: u32,
    ) -> Result<(), Error> {
        if !path.exists() {
            std::fs::create_dir(path)?;
        }

        for (name, ino) in T::list_sub_dir(&dir)? {
            let name = T::NameType::name_3ds_to_str(&name);
            for _ in 0..indent {
                print!(" ");
            }
            println!("+{}", &name);
            let dir = T::dir_open_ino(self.save.clone(), ino)?;
            self.extract_impl(dir, &path.join(name), indent + 1)?;
        }

        for (name, ino) in T::list_sub_file(&dir)? {
            let name = T::NameType::name_3ds_to_str(&name);
            for _ in 0..indent {
                print!(" ");
            }
            println!("-{}", &name);
            let file = T::file_open_ino(self.save.clone(), ino)?;
            let mut buffer = vec![0; T::len(&file)];
            match T::read(&file, 0, &mut buffer) {
                Ok(()) | Err(Error::HashMismatch) => (),
                e => return e,
            }
            std::fs::write(&path.join(name), &buffer)?;
        }

        Ok(())
    }

    fn extract(&self, mountpoint: &std::path::Path) -> Result<(), Error> {
        println!("Extracting...");
        let root = T::open_root(self.save.clone())?;
        self.extract_impl(root, mountpoint, 0)?;
        println!("Finished");
        Ok(())
    }

    fn clear_impl(&self, dir: &T::DirType) -> Result<(), Error> {
        for (_, ino) in T::list_sub_dir(&dir)? {
            let dir = T::dir_open_ino(self.save.clone(), ino)?;
            self.clear_impl(&dir)?;
            T::dir_delete(dir)?;
        }

        for (_, ino) in T::list_sub_file(&dir)? {
            let file = T::file_open_ino(self.save.clone(), ino)?;
            T::file_delete(file)?;
        }

        Ok(())
    }

    fn import_impl(&self, dir: &T::DirType, path: &std::path::Path) -> Result<(), Error> {
        for entry in std::fs::read_dir(&path)? {
            let entry = entry?;
            println!("{:?}", entry.path());
            let name = if let Some(name) = entry
                .path()
                .file_name()
                .and_then(OsStr::to_str)
                .and_then(T::NameType::name_str_to_3ds)
            {
                name
            } else {
                println!("Name not valid: {:?}", entry.path());
                continue;
            };

            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                let dir = T::new_sub_dir(&dir, name)?;
                self.import_impl(&dir, &entry.path())?
            } else if file_type.is_file() {
                let mut host_file = std::fs::File::open(&entry.path())?;
                let len = host_file.metadata()?.len() as usize;
                let file = T::new_sub_file(&dir, name, len)?;
                let mut buffer = vec![0; len];
                host_file.read_exact(&mut buffer)?;
                T::write(&file, 0, &buffer)?;
                T::commit_file(&file)?;
            } else {
                println!("Unrecognized file type: {:?}", entry.path());
            }
        }

        Ok(())
    }

    fn import(&self, mountpoint: &std::path::Path) -> Result<(), Error> {
        println!("Clearing the original contents...");
        let root = T::open_root(self.save.clone())?;
        self.clear_impl(&root)?;
        println!("Importing new contents...");
        self.import_impl(&root, mountpoint)?;
        T::commit(&self.save)?;
        println!("Finished");
        Ok(())
    }

    #[allow(unreachable_code, unused_variables, unused_mut)]
    fn mount(mut self, read_only: bool, mountpoint: &std::path::Path) -> Result<(), Error> {
        #[cfg(all(unix, feature = "unixfuse"))]
        {
            self.read_only = read_only;
            mount(self, &mountpoint, &[])?;
            return Ok(());
        }
        println!("fuse not implemented. Please specify --extract or --import flag");
        Ok(())
    }

    fn start(
        self,
        operation: FileSystemOperation,
        mountpoint: &std::path::Path,
    ) -> Result<(), Error> {
        match operation {
            FileSystemOperation::Mount(read_only) => self.mount(read_only, mountpoint)?,
            FileSystemOperation::Extract => self.extract(mountpoint)?,
            FileSystemOperation::Import => self.import(mountpoint)?,
        }

        Ok(())
    }
}

#[cfg(all(unix, feature = "unixfuse"))]
fn make_dir_attr(read_only: bool, uid: u32, gid: u32, ino: u64, sub_file_count: usize) -> FileAttr {
    FileAttr {
        ino,
        size: 0,
        blocks: 0,
        atime: time::Timespec::new(0, 0),
        mtime: time::Timespec::new(0, 0),
        ctime: time::Timespec::new(0, 0),
        crtime: time::Timespec::new(0, 0),
        kind: FileType::Directory,
        perm: if read_only { 0o555 } else { 0o755 },
        nlink: 2 + sub_file_count as u32,
        uid: uid,
        gid: gid,
        rdev: 0,
        flags: 0,
    }
}

#[cfg(all(unix, feature = "unixfuse"))]
fn make_file_attr(read_only: bool, uid: u32, gid: u32, ino: u64, file_size: usize) -> FileAttr {
    FileAttr {
        ino,
        size: file_size as u64,
        blocks: 1,
        atime: time::Timespec::new(0, 0),
        mtime: time::Timespec::new(0, 0),
        ctime: time::Timespec::new(0, 0),
        crtime: time::Timespec::new(0, 0),
        kind: FileType::RegularFile,
        perm: if read_only { 0o444 } else { 0o644 },
        nlink: 1,
        uid: uid,
        gid: gid,
        rdev: 0,
        flags: 0,
    }
}

fn is_legal_char(c: u8) -> bool {
    c >= 32 && c < 127 && c != 47 && c != 92
}

trait NameConvert {
    fn name_3ds_to_str(name: &Self) -> String;
    fn name_str_to_3ds(name: &str) -> Option<Self>
    where
        Self: Sized;
}

fn name_os_to_3ds<T: NameConvert>(name: &OsStr) -> Option<(T, &str)> {
    let s = name.to_str()?;
    let argument_pos = s.find("\\+");
    let (l, r) = if let Some(pos) = argument_pos {
        let (l, r) = s.split_at(pos);
        (l, &r[2..])
    } else {
        (s, "")
    };
    Some((T::name_str_to_3ds(l)?, r))
}

impl NameConvert for u64 {
    fn name_3ds_to_str(name: &u64) -> String {
        format!("{:016x}", name)
    }

    fn name_str_to_3ds(name: &str) -> Option<u64> {
        u64::from_str_radix(name, 16).ok()
    }
}

impl NameConvert for [u8; 16] {
    fn name_3ds_to_str(name: &[u8; 16]) -> String {
        let mut last_char = 15;
        loop {
            if name[last_char] != 0 || last_char == 0 {
                break;
            }
            last_char -= 1;
        }

        name[0..=last_char]
            .iter()
            .map(|x| {
                if is_legal_char(*x) {
                    String::from_utf8(vec![*x]).unwrap()
                } else {
                    format!("\\x{:02x}", *x)
                }
            })
            .fold("".to_owned(), |mut x, y| {
                x.push_str(&y);
                x
            })
    }

    fn name_str_to_3ds(name: &str) -> Option<[u8; 16]> {
        let mut name_converted = [0; 16];
        let bytes = name.as_bytes();
        let mut out_i = 0;
        let mut in_i = 0;
        loop {
            if in_i == bytes.len() {
                break;
            }
            if out_i == name_converted.len() {
                return None;
            }

            if bytes[in_i] != b'\\' {
                name_converted[out_i] = bytes[in_i];
                out_i += 1;
                in_i += 1;
            } else {
                in_i += 1;
                if *bytes.get(in_i)? != b'x' {
                    return None;
                }
                in_i += 1;
                name_converted[out_i] =
                    u8::from_str_radix(std::str::from_utf8(bytes.get(in_i..in_i + 2)?).ok()?, 16)
                        .ok()?;
                out_i += 1;
                in_i += 2;
            }
        }
        Some(name_converted)
    }
}

#[cfg(all(unix, feature = "unixfuse"))]
enum Ino {
    Dir(u32),
    File(u32),
}

#[cfg(all(unix, feature = "unixfuse"))]
impl Ino {
    fn to_os(&self) -> u64 {
        match *self {
            Ino::Dir(ino) => u64::from(ino),
            Ino::File(ino) => u64::from(ino) + 0x1_0000_0000,
        }
    }

    fn from_os(ino: u64) -> Ino {
        if ino >= 0x1_0000_0000 {
            Ino::File((ino - 0x1_0000_0000) as u32)
        } else {
            Ino::Dir(ino as u32)
        }
    }
}

impl<T: file_system::FileSystem> Drop for FileSystemFrontend<T> {
    fn drop(&mut self) {
        if !self.read_only {
            T::commit(self.save.as_ref()).unwrap();
            println!("Saved");
        }
    }
}

#[cfg(all(unix, feature = "unixfuse"))]
impl<T: file_system::FileSystem> Filesystem for FileSystemFrontend<T>
where
    T::NameType: NameConvert + Clone,
{
    fn init(&mut self, _req: &Request) -> Result<(), i32> {
        let (uid, gid) = unsafe { (geteuid(), getegid()) };
        self.uid = uid;
        self.gid = gid;
        println!("Initialized");
        Ok(())
    }

    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let name_converted: T::NameType = if let Some((n, _)) = name_os_to_3ds(name) {
            n
        } else {
            reply.error(ENAMETOOLONG);
            return;
        };

        match Ino::from_os(parent) {
            Ino::File(_) => {
                reply.error(ENOTDIR);
            }
            Ino::Dir(ino) => {
                let parent_dir = if let Ok(parent_dir) = T::dir_open_ino(self.save.clone(), ino) {
                    parent_dir
                } else {
                    reply.error(EIO);
                    return;
                };

                if let Ok(child) = T::open_sub_dir(&parent_dir, name_converted.clone()) {
                    let children_len = if let Ok(chidren) = T::list_sub_dir(&child) {
                        chidren.len()
                    } else {
                        reply.error(EIO);
                        return;
                    };

                    reply.entry(
                        &time::Timespec::new(1, 0),
                        &make_dir_attr(
                            self.read_only,
                            self.uid,
                            self.gid,
                            Ino::Dir(T::dir_get_ino(&child)).to_os(),
                            children_len,
                        ),
                        0,
                    );
                    return;
                }
                if let Ok(child) = T::open_sub_file(&parent_dir, name_converted) {
                    reply.entry(
                        &time::Timespec::new(1, 0),
                        &make_file_attr(
                            self.read_only,
                            self.uid,
                            self.gid,
                            Ino::File(T::file_get_ino(&child)).to_os(),
                            T::len(&child),
                        ),
                        0,
                    );
                    return;
                }
                reply.error(ENOENT);
            }
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        match Ino::from_os(ino) {
            Ino::File(ino) => {
                if let Ok(file) = T::file_open_ino(self.save.clone(), ino) {
                    reply.attr(
                        &time::Timespec::new(1, 0),
                        &make_file_attr(
                            self.read_only,
                            self.uid,
                            self.gid,
                            Ino::File(T::file_get_ino(&file)).to_os(),
                            T::len(&file),
                        ),
                    );
                } else {
                    reply.error(ENOENT);
                }
            }
            Ino::Dir(ino) => {
                if let Ok(dir) = T::dir_open_ino(self.save.clone(), ino) {
                    let children_len = if let Ok(chidren) = T::list_sub_dir(&dir) {
                        chidren.len()
                    } else {
                        reply.error(EIO);
                        return;
                    };
                    reply.attr(
                        &time::Timespec::new(1, 0),
                        &make_dir_attr(
                            self.read_only,
                            self.uid,
                            self.gid,
                            Ino::Dir(T::dir_get_ino(&dir)).to_os(),
                            children_len,
                        ),
                    );
                } else {
                    reply.error(ENOENT);
                }
            }
        }
    }

    fn setattr(
        &mut self,
        _req: &Request,
        ino: u64,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<time::Timespec>,
        _mtime: Option<time::Timespec>,
        fh: Option<u64>,
        _crtime: Option<time::Timespec>,
        _chgtime: Option<time::Timespec>,
        _bkuptime: Option<time::Timespec>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        match Ino::from_os(ino) {
            Ino::File(ino) => {
                let mut file_holder: Option<T::FileType>;
                let file = if let Some(fh) = fh {
                    if let Some(file) = self.fh_map.get_mut(&fh) {
                        file
                    } else {
                        reply.error(ENOENT);
                        return;
                    }
                } else if let Some(file) = self
                    .fh_map
                    .iter_mut()
                    .filter(|(_, b)| T::file_get_ino(b) == ino)
                    .map(|(_, b)| b)
                    .next()
                {
                    // bash stdout redirection would do this when the dest file exists
                    // TODO: revisit this when implementing safe multi fh
                    println!("Warning: resize when another fh is opened.");
                    file
                } else if let Ok(file) = T::file_open_ino(self.save.clone(), ino) {
                    file_holder = Some(file);
                    file_holder.as_mut().unwrap()
                } else {
                    reply.error(ENOENT);
                    return;
                };

                if let Some(size) = size {
                    match T::resize(file, size as usize) {
                        Ok(()) => reply.attr(
                            &time::Timespec::new(1, 0),
                            &make_file_attr(
                                self.read_only,
                                self.uid,
                                self.gid,
                                Ino::File(T::file_get_ino(&file)).to_os(),
                                T::len(&file),
                            ),
                        ),
                        Err(_) => reply.error(EIO),
                    }
                }
            }
            Ino::Dir(_) => reply.error(ENOSYS),
        }
    }

    fn mkdir(&mut self, _req: &Request, parent: u64, name: &OsStr, _mode: u32, reply: ReplyEntry) {
        if self.read_only {
            reply.error(EROFS);
            return;
        }
        let name_converted: T::NameType = if let Some((n, _)) = name_os_to_3ds(name) {
            n
        } else {
            reply.error(ENAMETOOLONG);
            return;
        };
        match Ino::from_os(parent) {
            Ino::File(_) => {
                reply.error(ENOTDIR);
            }
            Ino::Dir(ino) => {
                let parent_dir = if let Ok(parent_dir) = T::dir_open_ino(self.save.clone(), ino) {
                    parent_dir
                } else {
                    reply.error(EIO);
                    return;
                };
                match T::new_sub_dir(&parent_dir, name_converted) {
                    Ok(child) => reply.entry(
                        &time::Timespec::new(1, 0),
                        &make_dir_attr(
                            self.read_only,
                            self.uid,
                            self.gid,
                            Ino::Dir(T::dir_get_ino(&child)).to_os(),
                            0,
                        ),
                        0,
                    ),
                    Err(Error::AlreadyExist) => reply.error(EEXIST),
                    Err(Error::NoSpace) => reply.error(ENOSPC),
                    Err(_) => reply.error(EIO),
                }
                return;
            }
        }
    }

    fn mknod(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _rdev: u32,
        reply: ReplyEntry,
    ) {
        if self.read_only {
            reply.error(EROFS);
            return;
        }
        let (name_converted, size): (T::NameType, usize) =
            if let Some((n, s)) = name_os_to_3ds(name) {
                (n, str::parse::<usize>(s).unwrap_or(0))
            } else {
                reply.error(ENAMETOOLONG);
                return;
            };
        match Ino::from_os(parent) {
            Ino::File(_) => {
                reply.error(ENOTDIR);
            }
            Ino::Dir(ino) => {
                let parent_dir = if let Ok(parent_dir) = T::dir_open_ino(self.save.clone(), ino) {
                    parent_dir
                } else {
                    reply.error(EIO);
                    return;
                };

                match T::new_sub_file(&parent_dir, name_converted, size) {
                    Ok(child) => reply.entry(
                        &time::Timespec::new(1, 0),
                        &make_file_attr(
                            self.read_only,
                            self.uid,
                            self.gid,
                            Ino::File(T::file_get_ino(&child)).to_os(),
                            0,
                        ),
                        0,
                    ),
                    Err(Error::AlreadyExist) => reply.error(EEXIST),
                    Err(Error::NoSpace) => reply.error(ENOSPC),
                    Err(_) => reply.error(EIO),
                }
                return;
            }
        }
    }

    fn rmdir(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        if self.read_only {
            reply.error(EROFS);
            return;
        }
        let name_converted: T::NameType = if let Some((n, _)) = name_os_to_3ds(name) {
            n
        } else {
            reply.error(ENAMETOOLONG);
            return;
        };

        match Ino::from_os(parent) {
            Ino::File(_) => {
                reply.error(ENOTDIR);
            }
            Ino::Dir(ino) => {
                let parent_dir = if let Ok(parent_dir) = T::dir_open_ino(self.save.clone(), ino) {
                    parent_dir
                } else {
                    reply.error(EIO);
                    return;
                };

                if let Ok(child) = T::open_sub_dir(&parent_dir, name_converted) {
                    match T::dir_delete(child) {
                        Ok(()) => reply.ok(),
                        Err(Error::NotEmpty) => reply.error(ENOTEMPTY),
                        Err(_) => reply.error(EIO),
                    }
                    return;
                }
                reply.error(ENOENT);
            }
        }
    }

    fn unlink(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        if self.read_only {
            reply.error(EROFS);
            return;
        }
        let name_converted: T::NameType = if let Some((n, _)) = name_os_to_3ds(name) {
            n
        } else {
            reply.error(ENAMETOOLONG);
            return;
        };

        match Ino::from_os(parent) {
            Ino::File(_) => {
                reply.error(ENOTDIR);
            }
            Ino::Dir(ino) => {
                let parent_dir = if let Ok(parent_dir) = T::dir_open_ino(self.save.clone(), ino) {
                    parent_dir
                } else {
                    reply.error(EIO);
                    return;
                };

                if let Ok(child) = T::open_sub_file(&parent_dir, name_converted) {
                    match T::file_delete(child) {
                        Ok(()) => reply.ok(),
                        Err(_) => reply.error(EIO),
                    }
                    return;
                }
                reply.error(ENOENT);
            }
        }
    }

    fn open(&mut self, _req: &Request, ino: u64, _flags: u32, reply: ReplyOpen) {
        match Ino::from_os(ino) {
            Ino::File(ino) => {
                if let Ok(file) = T::file_open_ino(self.save.clone(), ino) {
                    self.fh_map.insert(self.next_fh, file);
                    reply.opened(self.next_fh, 0);
                    self.next_fh += 1;
                } else {
                    reply.error(ENOENT);
                }
            }
            Ino::Dir(_) => {
                reply.error(EISDIR);
            }
        }
    }

    fn release(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        _flags: u32,
        _lock_owner: u64,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        if let Some(file) = self.fh_map.remove(&fh) {
            if !self.read_only {
                if let Err(e) = T::commit_file(&file) {
                    println!("Failed to save file: {}", e);
                }
            }
        }
        reply.ok();
    }

    fn read(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        reply: ReplyData,
    ) {
        let offset = offset as usize;
        let size = size as usize;
        if let Some(file) = self.fh_map.get(&fh) {
            if size == 0 {
                reply.data(&[]);
                return;
            }
            let end = std::cmp::min(offset + size, T::len(&file));
            if end <= offset {
                reply.data(&[]);
                return;
            }
            let mut buf = vec![0; end - offset];
            match T::read(&file, offset, &mut buf) {
                Ok(()) | Err(Error::HashMismatch) => reply.data(&buf),
                _ => reply.error(EIO),
            }
        } else {
            reply.error(EBADF);
        }
    }

    fn write(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _flags: u32,
        reply: ReplyWrite,
    ) {
        if self.read_only {
            reply.error(EROFS);
            return;
        }

        let offset = offset as usize;
        let end = offset + data.len();
        if let Some(mut file) = self.fh_map.get_mut(&fh) {
            if data.is_empty() {
                reply.written(0);
                return;
            }
            if end > T::len(&file) {
                match T::resize(&mut file, end) {
                    Ok(()) => (),
                    Err(Error::NoSpace) => {
                        reply.error(ENOSPC);
                        return;
                    }
                    Err(_) => {
                        reply.error(EIO);
                        return;
                    }
                }
                match T::write(&file, offset, &data) {
                    Ok(()) => reply.written(data.len() as u32),
                    _ => reply.error(EIO),
                }
            }
        } else {
            reply.error(EBADF);
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        match Ino::from_os(ino) {
            Ino::File(_) => reply.error(ENOTDIR),
            Ino::Dir(ino) => {
                if let Ok(dir) = T::dir_open_ino(self.save.clone(), ino) {
                    let parent_ino = if ino == 1 {
                        1
                    } else {
                        T::dir_get_parent_ino(&dir)
                    };
                    let mut entries = vec![
                        (Ino::Dir(ino).to_os(), FileType::Directory, ".".to_owned()),
                        (
                            Ino::Dir(parent_ino).to_os(),
                            FileType::Directory,
                            "..".to_owned(),
                        ),
                    ];

                    let sub_dirs = if let Ok(r) = T::list_sub_dir(&dir) {
                        r
                    } else {
                        reply.error(EIO);
                        return;
                    };
                    for (name, i) in sub_dirs {
                        entries.push((
                            Ino::Dir(i).to_os(),
                            FileType::Directory,
                            T::NameType::name_3ds_to_str(&name),
                        ));
                    }

                    let sub_files = if let Ok(r) = T::list_sub_file(&dir) {
                        r
                    } else {
                        reply.error(EIO);
                        return;
                    };
                    for (name, i) in sub_files {
                        entries.push((
                            Ino::File(i).to_os(),
                            FileType::RegularFile,
                            T::NameType::name_3ds_to_str(&name),
                        ));
                    }

                    let to_skip = if offset == 0 { offset } else { offset + 1 } as usize;
                    for (i, entry) in entries.into_iter().enumerate().skip(to_skip) {
                        reply.add(entry.0, i as i64, entry.1, entry.2);
                    }
                    reply.ok();
                } else {
                    reply.error(ENOENT);
                }
            }
        }
    }

    fn rename(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        reply: ReplyEmpty,
    ) {
        if self.read_only {
            reply.error(EROFS);
            return;
        }

        let name_converted: T::NameType = if let Some((n, _)) = name_os_to_3ds(name) {
            n
        } else {
            reply.error(ENAMETOOLONG);
            return;
        };
        let newname_converted: T::NameType = if let Some((n, _)) = name_os_to_3ds(newname) {
            n
        } else {
            reply.error(ENAMETOOLONG);
            return;
        };

        let dir = match Ino::from_os(parent) {
            Ino::File(_) => {
                reply.error(ENOTDIR);
                return;
            }
            Ino::Dir(ino) => match T::dir_open_ino(self.save.clone(), ino) {
                Ok(dir) => dir,
                Err(_) => {
                    reply.error(EIO);
                    return;
                }
            },
        };

        let newdir = match Ino::from_os(newparent) {
            Ino::File(_) => {
                reply.error(ENOTDIR);
                return;
            }
            Ino::Dir(ino) => match T::dir_open_ino(self.save.clone(), ino) {
                Ok(dir) => dir,
                Err(_) => {
                    reply.error(EIO);
                    return;
                }
            },
        };

        if let Ok(mut file) = T::open_sub_file(&dir, name_converted.clone()) {
            if let Ok(old_file) = T::open_sub_file(&newdir, newname_converted.clone()) {
                match T::file_delete(old_file) {
                    Ok(()) => (),
                    Err(_) => {
                        reply.error(EIO);
                        return;
                    }
                }
            }

            match T::file_rename(&mut file, &newdir, newname_converted.clone()) {
                Ok(()) => reply.ok(),
                Err(Error::AlreadyExist) => reply.error(EEXIST),
                Err(_) => reply.error(EIO),
            }
        } else if let Ok(mut dir) = T::open_sub_dir(&dir, name_converted.clone()) {
            if let Ok(old_dir) = T::open_sub_dir(&newdir, newname_converted.clone()) {
                match T::dir_delete(old_dir) {
                    Ok(()) => (),
                    Err(Error::NotEmpty) => {
                        reply.error(ENOTEMPTY);
                        return;
                    }
                    Err(_) => {
                        reply.error(EIO);
                        return;
                    }
                }
            }

            match T::dir_rename(&mut dir, &newdir, newname_converted) {
                Ok(()) => reply.ok(),
                Err(Error::AlreadyExist) => reply.error(EEXIST),
                Err(_) => reply.error(EIO),
            }
        } else {
            reply.error(ENOENT);
        }
    }
}

fn print_usage(program: &str, opts: Options) {
    let brief = format!("Usage: {} [OPTIONS] MOUNT_PATH", program);
    print!("{}", opts.usage(&brief));
}

fn get_default_bucket(n: usize) -> usize {
    if n < 3 {
        3
    } else if n < 19 {
        n | 1
    } else {
        let mut count = n;
        while count % 2 == 0
            || count % 3 == 0
            || count % 5 == 0
            || count % 7 == 0
            || count % 11 == 0
            || count % 13 == 0
            || count % 17 == 0
        {
            count += 1;
        }
        count
    }
}

fn to_ext_data_format_param(
    raw: HashMap<String, String>,
) -> Result<ExtDataFormatParam, Box<std::error::Error>> {
    let max_dir = raw
        .get("max_dir")
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(100);

    let dir_buckets = raw
        .get("dir_buckets")
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or_else(|| get_default_bucket(max_dir));

    let max_file = raw
        .get("max_file")
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(100);

    let file_buckets = raw
        .get("file_buckets")
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or_else(|| get_default_bucket(max_file));

    Ok(ExtDataFormatParam {
        max_dir,
        dir_buckets,
        max_file,
        file_buckets,
    })
}

fn to_save_data_format_param(
    raw: HashMap<String, String>,
) -> Result<(SaveDataFormatParam, usize), Box<std::error::Error>> {
    let block_len = raw
        .get("block_len")
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(512);

    let block_type = match block_len {
        512 => SaveDataBlockType::Small,
        4096 => SaveDataBlockType::Large,
        _ => {
            println!("Unsupported block_len value");
            return Err(Box::from(Error::InvalidValue));
        }
    };

    let max_dir = raw
        .get("max_dir")
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(100);

    let dir_buckets = raw
        .get("dir_buckets")
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or_else(|| get_default_bucket(max_dir));

    let max_file = raw
        .get("max_file")
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(100);

    let file_buckets = raw
        .get("file_buckets")
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or_else(|| get_default_bucket(max_file));

    let duplicate_data = raw
        .get("duplicate_data")
        .map(|s| s.parse::<bool>())
        .transpose()?
        .unwrap_or(true);

    let len = raw
        .get("len")
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(512 * 1024);

    Ok((
        SaveDataFormatParam {
            block_type,
            max_dir,
            dir_buckets,
            max_file,
            file_buckets,
            duplicate_data,
        },
        len,
    ))
}

fn main() -> Result<(), Box<std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let program = args[0].clone();

    let mut opts = Options::new();
    opts.optopt("b", "boot9", "boot9.bin file path", "DIR");
    opts.optflag("h", "help", "print this help menu");
    opts.optflag("i", "import", "import the content instead of mounting");
    opts.optopt("m", "movable", "movable.sed file path", "FILE");
    opts.optflag("r", "readonly", "mount as read-only file system");
    opts.optopt("", "bare", "mount a bare DISA file", "FILE");
    opts.optopt(
        "",
        "db",
        "mount a database. DB_TYPE is one of the following:
    nandtitle, nandimport, tmptitle, tmpimport, sdtitle, sdimport, ticket",
        "DB_TYPE",
    );
    opts.optopt(
        "f",
        "format",
        "format the specified archive",
        "[\"\"|param1:value1[,...]]",
    );
    opts.optopt("", "sd", "SD root path", "DIR");
    opts.optopt("", "sdext", "mount the SD Extdata with the ID", "ID");
    opts.optopt("", "sdsave", "mount the SD save with the ID", "ID");
    opts.optopt("", "nand", "NAND root path", "DIR");
    opts.optopt("", "nandext", "mount the NAND Extdata with the ID", "ID");
    opts.optopt("", "nandsave", "mount the NAND save with the ID", "ID");
    opts.optopt("o", "otp", "OTP file path", "FILE");
    opts.optflag("x", "extract", "extract the content instead of mounting");

    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(f) => {
            println!("Failed to parse the arguments: {}", f);
            print_usage(&program, opts);
            return Ok(());
        }
    };

    if matches.opt_present("h") {
        print_usage(&program, opts);
        return Ok(());
    }

    if matches.free.len() != 1 {
        println!("Please specify one mount path");
        return Ok(());
    }

    let mountpoint = std::path::Path::new(&matches.free[0]);

    let boot9_path = matches.opt_str("boot9");
    let movable_path = matches.opt_str("movable");
    let otp_path = matches.opt_str("otp");
    let bare_path = matches.opt_str("bare");
    let sd_path = matches.opt_str("sd");
    let sd_save_id = matches.opt_str("sdsave");
    let sd_ext_id = matches.opt_str("sdext");
    let nand_path = matches.opt_str("nand");
    let nand_ext_id = matches.opt_str("nandext");
    let nand_save_id = matches.opt_str("nandsave");
    let db_type = matches.opt_str("db");
    let format_param = matches.opt_str("format");

    let format_param: Option<HashMap<String, String>> = format_param.map(|s| {
        s.split(',')
            .filter_map(|p| {
                if let Some(mid) = p.find(':') {
                    let (l, r) = p.split_at(mid);
                    Some((l.to_owned(), r[1..].to_owned()))
                } else {
                    None
                }
            })
            .collect()
    });

    let read_only = matches.opt_present("r") || matches.opt_present("extract");

    let operation = if matches.opt_present("extract") {
        if matches.opt_present("import") {
            println!("--extract and --import at the same time not allowed");
            return Ok(());
        }
        FileSystemOperation::Extract
    } else if matches.opt_present("import") {
        FileSystemOperation::Import
    } else {
        FileSystemOperation::Mount(read_only)
    };

    if [
        &sd_save_id,
        &sd_ext_id,
        &nand_save_id,
        &nand_ext_id,
        &bare_path,
        &db_type,
    ]
    .iter()
    .map(|x| if x.is_none() { 0 } else { 1 })
    .sum::<i32>()
        != 1
    {
        println!(
            "One and only one of the following arguments must be supplied:
    --sdext, --sdsave, --nandsave, --nandext, --bare, --db"
        );
        return Ok(());
    }

    let resource = Resource::new(boot9_path, movable_path, sd_path, nand_path, otp_path)?;

    if let Some(bare) = bare_path {
        if let Some(format_param) = format_param {
            println!("Formatting...");
            let (param, len) = to_save_data_format_param(format_param)?;
            resource.format_bare_save(&bare, &param, len)?;
            println!("Formatting done");
        }

        println!(
            "WARNING: After modification, you need to sign the CMAC header using other tools."
        );

        FileSystemFrontend::<SaveDataFileSystem>::new(resource.open_bare_save(&bare, !read_only)?)
            .start(operation, mountpoint)?
    } else if let Some(id) = nand_save_id {
        let id = u32::from_str_radix(&id, 16)?;
        if let Some(format_param) = format_param {
            println!("Formatting...");
            let (param, len) = to_save_data_format_param(format_param)?;
            resource.format_nand_save(id, &param, len)?;
            println!("Formatting done");
        }

        FileSystemFrontend::<SaveDataFileSystem>::new(resource.open_nand_save(id, !read_only)?)
            .start(operation, mountpoint)?
    } else if let Some(id) = sd_save_id {
        let id = u64::from_str_radix(&id, 16)?;
        if let Some(format_param) = format_param {
            println!("Formatting...");
            let (param, len) = to_save_data_format_param(format_param)?;
            resource.format_sd_save(id, &param, len)?;
            println!("Formatting done");
        }

        FileSystemFrontend::<SaveDataFileSystem>::new(resource.open_sd_save(id, !read_only)?)
            .start(operation, mountpoint)?
    } else if let Some(id) = sd_ext_id {
        let id = u64::from_str_radix(&id, 16)?;
        if let Some(format_param) = format_param {
            println!("Formatting...");
            let param = to_ext_data_format_param(format_param)?;
            resource.format_sd_ext(id, &param)?;
            println!("Formatting done");
        }

        FileSystemFrontend::<ExtDataFileSystem>::new(resource.open_sd_ext(id, !read_only)?)
            .start(operation, mountpoint)?
    } else if let Some(id) = nand_ext_id {
        let id = u64::from_str_radix(&id, 16)?;

        FileSystemFrontend::<ExtDataFileSystem>::new(resource.open_nand_ext(id, !read_only)?)
            .start(operation, mountpoint)?
    } else if let Some(db_type) = db_type {
        let db_type = match db_type.as_ref() {
            "nandtitle" => DbType::NandTitle,
            "nandimport" => DbType::NandImport,
            "tmptitle" => DbType::TmpTitle,
            "tmpimport" => DbType::TmpImport,
            "sdtitle" => DbType::SdTitle,
            "sdimport" => DbType::SdImport,
            "ticket" => DbType::Ticket,
            _ => {
                println!("Unknown database type {}", db_type);
                return Ok(());
            }
        };

        FileSystemFrontend::<DbFileSystem>::new(resource.open_db(db_type, !read_only)?)
            .start(operation, mountpoint)?
    } else {
        panic!()
    };
    Ok(())
}

#[cfg(test)]
mod test {
    use crate::*;

    #[test]
    fn test_string_conversion() {
        assert_eq!(
            <[u8; 16]>::name_3ds_to_str(&[b'a', b'b', b'c', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            "abc"
        );

        assert_eq!(
            <[u8; 16]>::name_3ds_to_str(&[
                b'a', b'b', b'c', 0, 0, 0, b'd', 0, 0, 0, 0, 0, 0, 0, 0, 0
            ]),
            "abc\\x00\\x00\\x00d"
        );

        assert_eq!(
            <[u8; 16]>::name_3ds_to_str(&[
                b'a', b'/', b'\n', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ]),
            "a\\x2f\\x0a"
        );

        assert_eq!(
            <[u8; 16]>::name_3ds_to_str(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            "\\x00"
        );

        assert_eq!(
            name_os_to_3ds::<[u8; 16]>(OsStr::new("abc")),
            Some((
                [b'a', b'b', b'c', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                ""
            ))
        );
        assert_eq!(
            name_os_to_3ds::<[u8; 16]>(OsStr::new("a\\x12c")),
            Some((
                [b'a', 0x12, b'c', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                ""
            ))
        );
        assert_eq!(
            name_os_to_3ds::<[u8; 16]>(OsStr::new("a\\x12\x34c")),
            Some((
                [b'a', 0x12, 0x34, b'c', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                ""
            ))
        );
        assert_eq!(
            name_os_to_3ds::<[u8; 16]>(OsStr::new("a\\x12\x34c\\+x\\yz")),
            Some((
                [b'a', 0x12, 0x34, b'c', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                "x\\yz"
            ))
        );
        assert_eq!(
            name_os_to_3ds::<[u8; 16]>(OsStr::new("a\\x12\x34c\\+")),
            Some((
                [b'a', 0x12, 0x34, b'c', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                ""
            ))
        );
        assert_eq!(name_os_to_3ds::<[u8; 16]>(OsStr::new("a\\2c")), None);
        assert_eq!(name_os_to_3ds::<[u8; 16]>(OsStr::new("a\\x1")), None);
        assert_eq!(name_os_to_3ds::<[u8; 16]>(OsStr::new("a\\x")), None);
        assert_eq!(name_os_to_3ds::<[u8; 16]>(OsStr::new("a\\")), None);
        assert!(name_os_to_3ds::<[u8; 16]>(OsStr::new("aaaaaaaaaaaaaaaa")).is_some());
        assert!(name_os_to_3ds::<[u8; 16]>(OsStr::new("aaaaaaaaaaaaaaaaa")).is_none());
    }
}
