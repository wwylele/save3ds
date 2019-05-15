use fuse::*;
use getopts::Options;
use libc::{EBADF, EEXIST, EIO, EISDIR, ENAMETOOLONG, ENOENT, ENOSPC, ENOTDIR, ENOTEMPTY, EROFS};
use libsave3ds::error::*;
use libsave3ds::save_data::*;
use libsave3ds::save_ext_common;
use libsave3ds::Resource;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::rc::Rc;
use time;

struct SaveExtFilesystem<T: save_ext_common::FileSystem> {
    save: Rc<T::CenterType>,
    fh_map: HashMap<u64, T::FileType>,
    next_fh: u64,
    read_only: bool,
}

impl<T: save_ext_common::FileSystem> SaveExtFilesystem<T> {
    fn new(save: Rc<T::CenterType>, read_only: bool) -> SaveExtFilesystem<T> {
        SaveExtFilesystem::<T> {
            save,
            fh_map: HashMap::new(),
            next_fh: 1,
            read_only,
        }
    }

    fn make_dir_attr(&self, ino: u64, sub_file_count: usize) -> FileAttr {
        FileAttr {
            ino,
            size: 0,
            blocks: 0,
            atime: time::Timespec::new(0, 0),
            mtime: time::Timespec::new(0, 0),
            ctime: time::Timespec::new(0, 0),
            crtime: time::Timespec::new(0, 0),
            kind: FileType::Directory,
            perm: if self.read_only { 0o555 } else { 0o777 },
            nlink: 2 + sub_file_count as u32,
            uid: 501,
            gid: 20,
            rdev: 0,
            flags: 0,
        }
    }

    fn make_file_attr(&self, ino: u64, file_size: usize) -> FileAttr {
        FileAttr {
            ino,
            size: file_size as u64,
            blocks: 1,
            atime: time::Timespec::new(0, 0),
            mtime: time::Timespec::new(0, 0),
            ctime: time::Timespec::new(0, 0),
            crtime: time::Timespec::new(0, 0),
            kind: FileType::RegularFile,
            perm: if self.read_only { 0o444 } else { 0o666 },
            nlink: 1,
            uid: 501,
            gid: 20,
            rdev: 0,
            flags: 0,
        }
    }
}

fn is_legal_char(c: u8) -> bool {
    c >= 32 && c < 127 && c != 47 && c != 92
}

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

fn name_os_to_3ds(name: &OsStr) -> Option<[u8; 16]> {
    let mut name_converted = [0; 16];
    let bytes = name.to_str()?.as_bytes();
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

enum Ino {
    Dir(u32),
    File(u32),
}

impl Ino {
    fn to_os(&self) -> u64 {
        match *self {
            Ino::Dir(ino) => u64::from(ino),
            Ino::File(ino) => u64::from(ino) + 0x1_0000_0000,
        }
    }

    fn from_os(ino: u64) -> Ino {
        if ino > 0x1_0000_0000 {
            Ino::File((ino - 0x1_0000_0000) as u32)
        } else {
            Ino::Dir(ino as u32)
        }
    }
}

impl<T: save_ext_common::FileSystem> Drop for SaveExtFilesystem<T> {
    fn drop(&mut self) {
        if !self.read_only {
            T::commit(self.save.as_ref()).unwrap();
            println!("Saved");
        }
    }
}

impl<T: save_ext_common::FileSystem> Filesystem for SaveExtFilesystem<T> {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let name_converted = if let Some(n) = name_os_to_3ds(name) {
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
                    let children_len = if let Ok(chidren) = T::list_sub_dir(&child) {
                        chidren.len()
                    } else {
                        reply.error(EIO);
                        return;
                    };

                    reply.entry(
                        &time::Timespec::new(1, 0),
                        &self.make_dir_attr(Ino::Dir(T::dir_get_ino(&child)).to_os(), children_len),
                        0,
                    );
                    return;
                }
                if let Ok(child) = T::open_sub_file(&parent_dir, name_converted) {
                    reply.entry(
                        &time::Timespec::new(1, 0),
                        &self.make_file_attr(
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
                        &self.make_file_attr(
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
                        &self.make_dir_attr(Ino::Dir(T::dir_get_ino(&dir)).to_os(), children_len),
                    );
                } else {
                    reply.error(ENOENT);
                }
            }
        }
    }

    fn mkdir(&mut self, _req: &Request, parent: u64, name: &OsStr, _mode: u32, reply: ReplyEntry) {
        if self.read_only {
            reply.error(EROFS);
            return;
        }
        let name_converted = if let Some(n) = name_os_to_3ds(name) {
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
                        &self.make_dir_attr(Ino::Dir(T::dir_get_ino(&child)).to_os(), 0),
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
        let name_converted = if let Some(n) = name_os_to_3ds(name) {
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

                match T::new_sub_file(&parent_dir, name_converted, 0) {
                    Ok(child) => reply.entry(
                        &time::Timespec::new(1, 0),
                        &self.make_file_attr(Ino::File(T::file_get_ino(&child)).to_os(), 0),
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
        let name_converted = if let Some(n) = name_os_to_3ds(name) {
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
        let name_converted = if let Some(n) = name_os_to_3ds(name) {
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
        self.fh_map.remove(&fh);
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
                            name_3ds_to_str(&name),
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
                            name_3ds_to_str(&name),
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

        let name_converted = if let Some(n) = name_os_to_3ds(name) {
            n
        } else {
            reply.error(ENAMETOOLONG);
            return;
        };
        let newname_converted = if let Some(n) = name_os_to_3ds(newname) {
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

        if let Ok(mut file) = T::open_sub_file(&dir, name_converted) {
            if let Ok(old_file) = T::open_sub_file(&newdir, newname_converted) {
                match T::file_delete(old_file) {
                    Ok(()) => (),
                    Err(_) => {
                        reply.error(EIO);
                        return;
                    }
                }
            }

            match T::file_rename(&mut file, &newdir, newname_converted) {
                Ok(()) => reply.ok(),
                Err(Error::AlreadyExist) => reply.error(EEXIST),
                Err(_) => reply.error(EIO),
            }
        } else if let Ok(mut dir) = T::open_sub_dir(&dir, name_converted) {
            if let Ok(old_dir) = T::open_sub_dir(&newdir, newname_converted) {
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

fn main() -> Result<(), Box<std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let program = args[0].clone();

    let mut opts = Options::new();
    opts.optopt("b", "boot9", "boot9.bin file path", "DIR");
    opts.optflag("h", "help", "print this help menu");
    opts.optopt("m", "movable", "movable.sed file path", "FILE");
    opts.optflag("r", "readonly", "mount as read-only file system");
    opts.optopt("", "bare", "mount a bare DISA file", "FILE");
    opts.optopt("", "sd", "SD root path", "DIR");
    opts.optopt("", "sdsave", "mount the SD save with the ID", "ID");
    opts.optopt("", "nand", "NAND root path", "DIR");
    opts.optopt("", "nandsave", "mount the NAND save with the ID", "ID");

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

    let boot9_path = matches.opt_str("boot9");
    let movable_path = matches.opt_str("movable");
    let bare_path = matches.opt_str("bare");
    let sd_path = matches.opt_str("sd");
    let sd_id = matches.opt_str("sdsave");
    let nand_path = matches.opt_str("nand");
    let nand_id = matches.opt_str("nandsave");

    if [&sd_id, &nand_id, &bare_path]
        .iter()
        .map(|x| if x.is_none() { 0 } else { 1 })
        .sum::<i32>()
        != 1
    {
        println!("One and only one of the following arguments must be supplied: --sdsave, --nandsave, --bare");
        return Ok(());
    }

    let resource = Resource::new(boot9_path, movable_path, sd_path, nand_path)?;

    let save = if let Some(bare) = bare_path {
        println!(
            "WARNING: After modification, you need to sign the CMAC header using other tools."
        );

        resource.open_bare_save(&bare)?
    } else if let Some(id) = nand_id {
        let id = u32::from_str_radix(&id, 16)?;
        resource.open_nand_save(id)?
    } else if let Some(id) = sd_id {
        let id = u64::from_str_radix(&id, 16)?;
        resource.open_sd_save(id)?
    } else {
        panic!()
    };

    let fs = SaveExtFilesystem::<SaveDataFileSystem>::new(save, matches.opt_present("r"));
    let options = [];
    let mountpoint = std::path::Path::new(&matches.free[0]);

    println!("Start mounting");
    mount(fs, &mountpoint, &options)?;
    Ok(())
}

#[cfg(test)]
mod test {
    use crate::*;

    #[test]
    fn test_string_conversion() {
        assert_eq!(
            name_3ds_to_str(&[b'a', b'b', b'c', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            "abc"
        );

        assert_eq!(
            name_3ds_to_str(&[b'a', b'b', b'c', 0, 0, 0, b'd', 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            "abc\\x00\\x00\\x00d"
        );

        assert_eq!(
            name_3ds_to_str(&[b'a', b'/', b'\n', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            "a\\x2f\\x0a"
        );

        assert_eq!(
            name_3ds_to_str(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            "\\x00"
        );

        assert_eq!(
            name_os_to_3ds(OsStr::new("abc")),
            Some([b'a', b'b', b'c', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
        );
        assert_eq!(
            name_os_to_3ds(OsStr::new("a\\x12c")),
            Some([b'a', 0x12, b'c', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
        );
        assert_eq!(
            name_os_to_3ds(OsStr::new("a\\x12\x34c")),
            Some([b'a', 0x12, 0x34, b'c', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
        );
        assert_eq!(name_os_to_3ds(OsStr::new("a\\2c")), None);
        assert_eq!(name_os_to_3ds(OsStr::new("a\\x1")), None);
        assert_eq!(name_os_to_3ds(OsStr::new("a\\x")), None);
        assert_eq!(name_os_to_3ds(OsStr::new("a\\")), None);
        assert!(name_os_to_3ds(OsStr::new("aaaaaaaaaaaaaaaa")).is_some());
        assert!(name_os_to_3ds(OsStr::new("aaaaaaaaaaaaaaaaa")).is_none());
    }
}
