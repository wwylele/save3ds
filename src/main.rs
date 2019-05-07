#![allow(dead_code)]

mod difi_partition;
mod disa;
mod disk_file;
mod dpfs_level;
mod dual_file;
mod fat;
mod ivfc_level;
mod memory_file;
mod random_access_file;
mod sub_file;

use disa::Disa;
use disk_file::DiskFile;
use memory_file::MemoryFile;
use random_access_file::RandomAccessFile;
use std::fs::File;
use std::io::prelude::*;
use std::rc::Rc;

fn main() {
    println!("Hello, world!");
    let test_file = File::open("/home/wwylele/save3ds/src/00000000.disa").unwrap();
    let file = Rc::new(DiskFile::new(test_file).unwrap());
    let disa = Disa::new(file).unwrap();
    for i in 0..disa.partition_count() {
        let file_name = format!("/home/wwylele/save3ds/dump-{}", i);
        let mut out = File::create(file_name).unwrap();
        let partition = &disa[i];
        for j in 0..partition.len() {
            let mut buf = [0xCC];
            partition.read(j, &mut buf);
            out.write_all(&buf).unwrap();
        }
    }
    println!("Done!");
}
