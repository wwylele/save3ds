use byte_struct::*;

#[derive(ByteStruct)]
#[byte_struct_le]
pub struct U32le {
    pub v: u32,
}

#[derive(ByteStruct)]
#[byte_struct_le]
pub struct U16le {
    pub v: u16,
}

#[derive(ByteStruct)]
#[byte_struct_le]
pub struct Magic {
    pub v: [u8; 4],
}
