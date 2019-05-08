#![allow(dead_code)]

mod difi_partition;
mod disa;
mod disk_file;
mod dpfs_level;
mod dual_file;
mod fat;
mod fs;
mod ivfc_level;
mod memory_file;
mod random_access_file;
mod save_data;
mod sub_file;

use disk_file::DiskFile;
use random_access_file::*;
use save_data::*;
use std::fs::File;
use std::io::*;
use std::rc::Rc;

fn traverse(indent: usize, dir: Dir, path: &str) {
    for name in dir.list_sub_dir().unwrap() {
        let trimmed: Vec<u8> = name.iter().cloned().take_while(|c| *c != 0).collect();
        let s = std::str::from_utf8(&trimmed).unwrap();
        for _ in 0..indent {
            print!("    ");
        }
        println!("+{}", s);
        let sub_path = path.to_owned() + "/" + s;
        std::fs::create_dir(&sub_path);
        traverse(indent + 1, dir.open_sub_dir(name).unwrap(), &sub_path);
    }
    for name in dir.list_sub_file().unwrap() {
        let trimmed: Vec<u8> = name.iter().cloned().take_while(|c| *c != 0).collect();
        let s = std::str::from_utf8(&trimmed).unwrap();
        for _ in 0..indent {
            print!("    ");
        }
        println!("-{}", s);
        let file = dir.open_sub_file(name).unwrap();
        let sub_path = path.to_owned() + "/" + s;
        let mut out_file = File::create(sub_path).unwrap();
        let mut buf = vec![0; file.len()];
        file.read(0, &mut buf);
        out_file.write_all(&buf).unwrap();
    }
}

fn main() {
    println!("Hello, world!");
    let test_file = File::open("/home/wwylele/save3ds/cecd").unwrap();
    let file = Rc::new(DiskFile::new(test_file).unwrap());
    let save = SaveData::new(file).unwrap();
    let root = Dir::open_root(save.clone()).unwrap();

    let dump_path = "/home/wwylele/save3ds/cecd_extract";
    //std::fs::remove_dir_all(dump_path);
    std::fs::create_dir(dump_path);
    println!("root");
    traverse(1, root, dump_path);
    println!("Done!");
}
