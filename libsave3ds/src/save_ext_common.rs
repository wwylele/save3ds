use crate::fs_meta::*;
use byte_struct::*;

#[derive(ByteStruct, Clone)]
#[byte_struct_le]
pub struct SaveExtDir {
    pub next: u32,
    pub sub_dir: u32,
    pub sub_file: u32,
    pub padding: u32,
}

#[derive(ByteStruct, Clone, PartialEq)]
#[byte_struct_le]
pub struct SaveExtKey {
    parent: u32,
    name: [u8; 16],
}

impl DirInfo for SaveExtDir {
    fn set_sub_dir(&mut self, index: u32) {
        self.sub_dir = index;
    }
    fn get_sub_dir(&self) -> u32 {
        self.sub_dir
    }
    fn set_sub_file(&mut self, index: u32) {
        self.sub_file = index;
    }
    fn get_sub_file(&self) -> u32 {
        self.sub_file
    }
    fn set_next(&mut self, index: u32) {
        self.next = index;
    }
    fn get_next(&self) -> u32 {
        self.next
    }
    fn new_root() -> Self {
        SaveExtDir {
            next: 0,
            sub_dir: 0,
            sub_file: 0,
            padding: 0,
        }
    }
}

impl ParentedKey for SaveExtKey {
    type NameType = [u8; 16];
    fn get_name(&self) -> [u8; 16] {
        self.name
    }
    fn get_parent(&self) -> u32 {
        self.parent
    }
    fn new(parent: u32, name: [u8; 16]) -> SaveExtKey {
        SaveExtKey { parent, name }
    }
}

#[cfg(test)]
mod test {
    use crate::save_ext_common::*;
    #[test]
    fn struct_size() {
        assert_eq!(FsInfo::BYTE_LEN, 0x68);
    }

}
