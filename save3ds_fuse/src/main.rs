use fuse::*;
use libc::{EBADF, EIO, EISDIR, ENOENT, ENOTDIR};
use libsave3ds::error::*;
use libsave3ds::save_data::*;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::rc::Rc;
use time;

struct SaveDataFilesystem {
    save: Rc<SaveData>,
    fh_map: HashMap<u64, File>,
    next_fh: u64,
}

impl SaveDataFilesystem {
    fn new(save: Rc<SaveData>) -> SaveDataFilesystem {
        SaveDataFilesystem {
            save,
            fh_map: HashMap::new(),
            next_fh: 1,
        }
    }
}

fn convert_name(name: &[u8; 16]) -> String {
    let trimmed: Vec<u8> = name.iter().cloned().take_while(|c| *c != 0).collect();
    std::str::from_utf8(&trimmed).unwrap().to_owned()
}

fn make_dir_attr(ino: u64, sub_file_count: usize) -> FileAttr {
    FileAttr {
        ino,
        size: 0,
        blocks: 0,
        atime: time::Timespec::new(0, 0),
        mtime: time::Timespec::new(0, 0),
        ctime: time::Timespec::new(0, 0),
        crtime: time::Timespec::new(0, 0),
        kind: FileType::Directory,
        perm: 0o777,
        nlink: 2 + sub_file_count as u32,
        uid: 501,
        gid: 20,
        rdev: 0,
        flags: 0,
    }
}

fn make_file_attr(ino: u64, file_size: usize) -> FileAttr {
    FileAttr {
        ino,
        size: file_size as u64,
        blocks: 1,
        atime: time::Timespec::new(0, 0),
        mtime: time::Timespec::new(0, 0),
        ctime: time::Timespec::new(0, 0),
        crtime: time::Timespec::new(0, 0),
        kind: FileType::RegularFile,
        perm: 0o666,
        nlink: 1,
        uid: 501,
        gid: 20,
        rdev: 0,
        flags: 0,
    }
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

impl Filesystem for SaveDataFilesystem {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        // TODO better name conversion
        let mut name_converted = [0; 16];
        let utf8 = name.to_str().unwrap().as_bytes();
        let len = std::cmp::min(16, utf8.len());
        name_converted[0..len].copy_from_slice(&utf8[0..len]);

        match Ino::from_os(parent) {
            Ino::File(_) => {
                reply.error(ENOTDIR);
            }
            Ino::Dir(ino) => {
                let parent_dir = if let Ok(parent_dir) = Dir::open_ino(self.save.clone(), ino) {
                    parent_dir
                } else {
                    reply.error(EIO);
                    return;
                };

                if let Ok(child) = parent_dir.open_sub_dir(name_converted) {
                    let children_len = if let Ok(chidren) = child.list_sub_dir() {
                        chidren.len()
                    } else {
                        reply.error(EIO);
                        return;
                    };

                    reply.entry(
                        &time::Timespec::new(1, 0),
                        &make_dir_attr(Ino::Dir(child.get_ino()).to_os(), children_len),
                        0,
                    );
                    return;
                }
                if let Ok(child) = parent_dir.open_sub_file(name_converted) {
                    reply.entry(
                        &time::Timespec::new(1, 0),
                        &make_file_attr(Ino::File(child.get_ino()).to_os(), child.len()),
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
                if let Ok(file) = File::open_ino(self.save.clone(), ino) {
                    reply.attr(
                        &time::Timespec::new(1, 0),
                        &make_file_attr(Ino::File(file.get_ino()).to_os(), file.len()),
                    );
                } else {
                    reply.error(ENOENT);
                }
            }
            Ino::Dir(ino) => {
                if let Ok(dir) = Dir::open_ino(self.save.clone(), ino) {
                    let children_len = if let Ok(chidren) = dir.list_sub_dir() {
                        chidren.len()
                    } else {
                        reply.error(EIO);
                        return;
                    };
                    reply.attr(
                        &time::Timespec::new(1, 0),
                        &make_dir_attr(Ino::Dir(dir.get_ino()).to_os(), children_len),
                    );
                } else {
                    reply.error(ENOENT);
                }
            }
        }
    }

    fn open(&mut self, _req: &Request, ino: u64, _flags: u32, reply: ReplyOpen) {
        match Ino::from_os(ino) {
            Ino::File(ino) => {
                if let Ok(file) = File::open_ino(self.save.clone(), ino) {
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
            let end = std::cmp::min(offset + size, file.len());
            if end <= offset {
                reply.data(&[]);
                return;
            }
            let mut buf = vec![0; end - offset];
            match file.read(offset, &mut buf) {
                Ok(()) | Err(Error::HashMismatch) => reply.data(&buf),
                _ => reply.error(EIO),
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
                if let Ok(dir) = Dir::open_ino(self.save.clone(), ino) {
                    let parent_ino = if ino == 1 { 1 } else { dir.get_parent_ino() };
                    let mut entries = vec![
                        (Ino::Dir(ino).to_os(), FileType::Directory, ".".to_owned()),
                        (
                            Ino::Dir(parent_ino).to_os(),
                            FileType::Directory,
                            "..".to_owned(),
                        ),
                    ];

                    let sub_dirs = if let Ok(r) = dir.list_sub_dir() {
                        r
                    } else {
                        reply.error(EIO);
                        return;
                    };
                    for (name, i) in sub_dirs {
                        entries.push((
                            Ino::Dir(i).to_os(),
                            FileType::Directory,
                            convert_name(&name),
                        ));
                    }

                    let sub_files = if let Ok(r) = dir.list_sub_file() {
                        r
                    } else {
                        reply.error(EIO);
                        return;
                    };
                    for (name, i) in sub_files {
                        entries.push((
                            Ino::File(i).to_os(),
                            FileType::RegularFile,
                            convert_name(&name),
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
}

fn main() {
    println!("Hello, world!");
    let file = std::fs::File::open("/home/wwylele/save3ds/cecd").unwrap();
    let save = SaveData::from_file(file).unwrap();

    let fs = SaveDataFilesystem::new(save);
    let options = [];
    let mountpoint = std::path::Path::new("/home/wwylele/save3ds/mount");
    mount(fs, &mountpoint, &options).unwrap();
}
