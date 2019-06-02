use crate::error::*;

pub trait FileSystemFile {
    type NameType;
    type DirType;

    fn rename(&mut self, parent: &Self::DirType, name: Self::NameType) -> Result<(), Error>;
    fn get_parent_ino(&self) -> Result<u32, Error>;
    fn get_ino(&self) -> u32;
    fn delete(self) -> Result<(), Error>;
    fn resize(&mut self, len: usize) -> Result<(), Error>;
    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error>;
    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn commit(&self) -> Result<(), Error>;
}

pub trait FileSystemDir {
    type NameType;
    type FileType;

    fn rename(&mut self, parent: &Self, name: Self::NameType) -> Result<(), Error>;
    fn get_parent_ino(&self) -> Result<u32, Error>;
    fn get_ino(&self) -> u32;
    fn open_sub_dir(&self, name: Self::NameType) -> Result<Self, Error>
    where
        Self: Sized;
    fn open_sub_file(&self, name: Self::NameType) -> Result<Self::FileType, Error>;
    fn list_sub_dir(&self) -> Result<Vec<(Self::NameType, u32)>, Error>;
    fn list_sub_file(&self) -> Result<Vec<(Self::NameType, u32)>, Error>;
    fn new_sub_dir(&self, name: Self::NameType) -> Result<Self, Error>
    where
        Self: Sized;
    fn new_sub_file(&self, name: Self::NameType, len: usize) -> Result<Self::FileType, Error>;
    fn delete(self) -> Result<(), Error>;
}

pub trait FileSystem {
    type FileType: FileSystemFile<NameType = Self::NameType, DirType = Self::DirType>;
    type DirType: FileSystemDir<NameType = Self::NameType, FileType = Self::FileType>;
    type NameType;

    fn open_file(&self, ino: u32) -> Result<Self::FileType, Error>;
    fn open_dir(&self, ino: u32) -> Result<Self::DirType, Error>;
    fn open_root(&self) -> Result<Self::DirType, Error> {
        self.open_dir(0)
    }
    fn commit(&self) -> Result<(), Error>;
}

#[cfg(test)]
#[allow(clippy::cyclomatic_complexity)]
pub mod test {
    fn is_one_prefix<T: PartialEq>(short: &[T], long: &[T]) -> bool {
        if short.len() + 1 != long.len() {
            return false;
        }
        is_prefix(short, long)
    }

    fn is_true_prefix<T: PartialEq>(short: &[T], long: &[T]) -> bool {
        if short.len() == long.len() {
            return false;
        }
        is_prefix(short, long)
    }

    fn is_prefix<T: PartialEq>(short: &[T], long: &[T]) -> bool {
        for (i, name) in short.iter().enumerate() {
            if long.get(i) != Some(name) {
                return false;
            }
        }
        true
    }

    struct DirMirror<T> {
        path: Vec<T>,
        ino: u32,
    }

    struct FileMirror<T> {
        path: Vec<T>,
        ino: u32,
        data: Vec<u8>,
    }

    use crate::file_system::*;

    pub fn fuzzer<T: FileSystem>(
        mut file_system: T,
        max_dir: usize,
        max_file: usize,
        reloader: impl Fn() -> T,
        gen_name: impl Fn() -> T::NameType,
        gen_len: impl Fn() -> usize,
    ) where
        T::NameType: Clone + PartialEq + Eq + std::hash::Hash + std::fmt::Debug,
    {
        use rand::distributions::Standard;
        use rand::prelude::*;
        use std::collections::HashSet;
        let mut rng = rand::thread_rng();

        let mut dir_mirrors: Vec<DirMirror<T::NameType>> = vec![DirMirror {
            path: vec![],
            ino: 1,
        }];

        let mut file_mirrors: Vec<FileMirror<T::NameType>> = vec![];

        for _ in 0..1000 {
            let main_op = rng.gen_range(0, 10);
            if main_op == 0 {
                // commit
                file_system.commit().unwrap();
            } else if main_op == 1 {
                // reload
                file_system.commit().unwrap();
                file_system = reloader();
            } else if main_op < 5 {
                // dir operations
                let dir_index = rng.gen_range(0, dir_mirrors.len());
                let dir_mirror = &dir_mirrors[dir_index];
                let mut dir = if rng.gen() {
                    // open via ino
                    file_system.open_dir(dir_mirror.ino).unwrap()
                } else {
                    // open via path
                    let mut current = file_system.open_dir(1).unwrap();
                    for name in dir_mirror.path.iter() {
                        current = current.open_sub_dir(name.clone()).unwrap();
                    }
                    current
                };

                // check ino info
                assert_eq!(dir.get_ino(), dir_mirror.ino);
                let parent_ino = dir.get_parent_ino().unwrap();
                if dir_mirror.ino == 1 {
                    assert_eq!(parent_ino, 0);
                } else {
                    let mut parent_path = dir_mirror.path.clone();
                    parent_path.pop().unwrap();
                    assert_eq!(
                        dir_mirrors
                            .iter()
                            .find(|d| d.path == parent_path)
                            .unwrap()
                            .ino,
                        parent_ino
                    );
                }

                // check sub dir
                let sub_dir_list: HashSet<_> = dir.list_sub_dir().unwrap().into_iter().collect();

                let sub_dir_mirror: HashSet<_> = dir_mirrors
                    .iter()
                    .filter(|d| is_one_prefix(&dir_mirror.path, &d.path))
                    .map(|d| (d.path.last().unwrap().clone(), d.ino))
                    .collect();

                assert_eq!(sub_dir_list, sub_dir_mirror);

                // check sub file
                let sub_file_list: HashSet<_> = dir.list_sub_file().unwrap().into_iter().collect();

                let sub_file_mirror: HashSet<_> = file_mirrors
                    .iter()
                    .filter(|d| is_one_prefix(&dir_mirror.path, &d.path))
                    .map(|d| (d.path.last().unwrap().clone(), d.ino))
                    .collect();

                assert_eq!(sub_file_list, sub_file_mirror);

                for _ in 0..10 {
                    let dir_mirror = &dir_mirrors[dir_index];
                    match rng.gen_range(0, 9) {
                        0..=2 => {
                            // new sub dir
                            let name = gen_name();
                            let mut child_path = dir_mirror.path.clone();
                            child_path.push(name.clone());
                            match dir.new_sub_dir(name) {
                                Err(Error::AlreadyExist) => {
                                    assert!(
                                        dir_mirrors.iter().any(|d| d.path == child_path)
                                            || file_mirrors.iter().any(|d| d.path == child_path)
                                    );
                                }
                                Err(Error::NoSpace) => {
                                    assert_eq!(dir_mirrors.len() - 1, max_dir);
                                }
                                Ok(child) => {
                                    assert!(dir_mirrors.iter().all(|d| d.path != child_path));
                                    assert!(file_mirrors.iter().all(|d| d.path != child_path));
                                    assert!(dir_mirrors.len() - 1 < max_dir);
                                    dir_mirrors.push(DirMirror {
                                        path: child_path,
                                        ino: child.get_ino(),
                                    })
                                }
                                _ => unreachable!(),
                            }
                        }
                        3 => {
                            // delete_dir
                            match dir.delete() {
                                Err(Error::DeletingRoot) => {
                                    assert_eq!(dir_mirror.ino, 1);
                                }
                                Err(Error::NotEmpty) => {
                                    assert!(
                                        dir_mirrors
                                            .iter()
                                            .any(|d| is_true_prefix(&dir_mirror.path, &d.path))
                                            || file_mirrors
                                                .iter()
                                                .any(|d| is_true_prefix(&dir_mirror.path, &d.path))
                                    );
                                }
                                Ok(()) => {
                                    assert!(dir_mirror.ino != 1);
                                    assert!(dir_mirrors
                                        .iter()
                                        .all(|d| !is_true_prefix(&dir_mirror.path, &d.path)));
                                    assert!(file_mirrors
                                        .iter()
                                        .all(|d| !is_true_prefix(&dir_mirror.path, &d.path)));
                                    dir_mirrors.remove(dir_index);
                                }
                                _ => unreachable!(),
                            }
                            break;
                        }
                        4..=5 => {
                            // rename dir
                            let new_parent_index = rng.gen_range(0, dir_mirrors.len());
                            let new_parent_mirror = &dir_mirrors[new_parent_index];
                            let new_name = gen_name();
                            if is_prefix(&dir_mirror.path, &new_parent_mirror.path) {
                                continue;
                            }
                            let new_parent = file_system.open_dir(new_parent_mirror.ino).unwrap();
                            if new_parent_mirror.ino == dir.get_parent_ino().unwrap()
                                && new_name == *dir_mirror.path.last().unwrap()
                            {
                                continue;
                            }

                            let old_path = dir_mirror.path.clone();
                            let mut new_path = new_parent_mirror.path.clone();
                            new_path.push(new_name.clone());
                            match dir.rename(&new_parent, new_name) {
                                Err(Error::AlreadyExist) => {
                                    assert!(
                                        dir_mirrors.iter().any(|d| d.path == new_path)
                                            || file_mirrors.iter().any(|d| d.path == new_path)
                                    );
                                }
                                Ok(()) => {
                                    assert!(dir_mirrors.iter().all(|d| d.path != new_path));
                                    assert!(file_mirrors.iter().all(|d| d.path != new_path));
                                    for child in dir_mirrors
                                        .iter_mut()
                                        .filter(|d| is_prefix(&old_path, &d.path))
                                    {
                                        child.path = new_path
                                            .iter()
                                            .chain(child.path.iter().skip(old_path.len()))
                                            .cloned()
                                            .collect();
                                    }
                                    for child in file_mirrors
                                        .iter_mut()
                                        .filter(|d| is_prefix(&old_path, &d.path))
                                    {
                                        child.path = new_path
                                            .iter()
                                            .chain(child.path.iter().skip(old_path.len()))
                                            .cloned()
                                            .collect();
                                    }
                                }
                                _ => unreachable!(),
                            }
                        }
                        6..=8 => {
                            // new sub file
                            let len = gen_len();
                            let name = gen_name();
                            let mut child_path = dir_mirror.path.clone();
                            child_path.push(name.clone());
                            match dir.new_sub_file(name, len) {
                                Err(Error::AlreadyExist) => {
                                    assert!(
                                        dir_mirrors.iter().any(|d| d.path == child_path)
                                            || file_mirrors.iter().any(|d| d.path == child_path)
                                    );
                                }
                                Err(Error::NoSpace) => {
                                    // assert_eq!(file_mirrors.len(), param.max_file as usize);
                                }
                                Ok(child) => {
                                    assert!(dir_mirrors.iter().all(|d| d.path != child_path));
                                    assert!(file_mirrors.iter().all(|d| d.path != child_path));
                                    assert!(file_mirrors.len() < max_file);
                                    let init: Vec<u8> =
                                        rng.sample_iter(&Standard).take(len).collect();
                                    if !init.is_empty() {
                                        child.write(0, &init).unwrap();
                                    }

                                    file_mirrors.push(FileMirror {
                                        path: child_path,
                                        ino: child.get_ino(),
                                        data: init,
                                    });
                                    child.commit().unwrap();
                                }
                                _ => unreachable!(),
                            }
                        }
                        _ => unreachable!(),
                    }
                }
            } else {
                // file operations
                if file_mirrors.is_empty() {
                    continue;
                }

                let file_index = rng.gen_range(0, file_mirrors.len());
                let mut file = if rng.gen() {
                    // open via ino
                    file_system.open_file(file_mirrors[file_index].ino).unwrap()
                } else {
                    // open via path
                    let mut current = file_system.open_dir(1).unwrap();
                    let mut path = file_mirrors[file_index].path.clone();
                    let file_name = path.pop().unwrap();
                    for name in path.iter() {
                        current = current.open_sub_dir(name.clone()).unwrap();
                    }
                    current.open_sub_file(file_name).unwrap()
                };

                // check ino info
                assert_eq!(file.get_ino(), file_mirrors[file_index].ino);
                let parent_ino = file.get_parent_ino().unwrap();
                let mut parent_path = file_mirrors[file_index].path.clone();
                parent_path.pop().unwrap();
                assert_eq!(
                    dir_mirrors
                        .iter()
                        .find(|d| d.path == parent_path)
                        .unwrap()
                        .ino,
                    parent_ino
                );

                for _ in 0..10 {
                    match rng.gen_range(0, 7) {
                        0 => {
                            // delete
                            file.delete().unwrap();
                            file_mirrors.remove(file_index);
                            break;
                        }
                        1 => {
                            // rename
                            let new_parent_index = rng.gen_range(0, dir_mirrors.len());
                            let new_parent_mirror = &dir_mirrors[new_parent_index];
                            let new_name = gen_name();
                            let new_parent = file_system.open_dir(new_parent_mirror.ino).unwrap();
                            if new_parent_mirror.ino == file.get_parent_ino().unwrap()
                                && new_name == *file_mirrors[file_index].path.last().unwrap()
                            {
                                continue;
                            }

                            let mut new_path = new_parent_mirror.path.clone();
                            new_path.push(new_name.clone());
                            match file.rename(&new_parent, new_name) {
                                Err(Error::AlreadyExist) => {
                                    assert!(
                                        dir_mirrors.iter().any(|d| d.path == new_path)
                                            || file_mirrors.iter().any(|d| d.path == new_path)
                                    );
                                }
                                Ok(()) => {
                                    assert!(dir_mirrors.iter().all(|d| d.path != new_path));
                                    assert!(file_mirrors.iter().all(|d| d.path != new_path));
                                    file_mirrors[file_index].path = new_path;
                                }
                                _ => unreachable!(),
                            }
                        }
                        2..=4 => {
                            // read/write
                            if file_mirrors[file_index].data.is_empty() {
                                continue;
                            }
                            let len = file_mirrors[file_index].data.len();
                            let pos = rng.gen_range(0, len);
                            let data_len = rng.gen_range(1, len - pos + 1);
                            if rng.gen() {
                                let a: Vec<u8> =
                                    rng.sample_iter(&Standard).take(data_len).collect();
                                file.write(pos, &a).unwrap();
                                file.commit().unwrap();
                                file_mirrors[file_index].data[pos..pos + data_len]
                                    .copy_from_slice(&a);
                            } else {
                                let mut a = vec![0; data_len];
                                file.read(pos, &mut a).unwrap();
                                assert_eq!(a, &file_mirrors[file_index].data[pos..pos + data_len]);
                            }
                        }
                        5 => {
                            assert_eq!(file_mirrors[file_index].data.len(), file.len());
                        }
                        6 => {
                            // resize
                            let old_len = file.len();
                            let len = gen_len();
                            match file.resize(len) {
                                Err(Error::NoSpace) => {
                                    //..
                                }
                                Ok(()) => {
                                    if len < old_len {
                                        file_mirrors[file_index].data.truncate(len);
                                    } else if len > old_len {
                                        let delta = len - old_len;
                                        let mut init: Vec<u8> =
                                            rng.sample_iter(&Standard).take(delta).collect();
                                        file.write(old_len, &init).unwrap();
                                        file_mirrors[file_index].data.append(&mut init);
                                    }
                                    file.commit().unwrap();
                                }
                                _ => unreachable!(),
                            }
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
    }
}
