use sha2::*;

pub fn hash_movable(key: [u8; 16]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(&key);
    let hash = hasher.finalize();
    let mut result = String::new();
    for index in &[3, 2, 1, 0, 7, 6, 5, 4, 11, 10, 9, 8, 15, 14, 13, 12] {
        result.extend(format!("{:02x}", hash[*index]).chars());
    }
    result
}

pub fn align_up(value: usize, align: usize) -> usize {
    value + (align - value % align) % align
}

pub fn divide_up(value: usize, align: usize) -> usize {
    if value == 0 {
        0
    } else {
        1 + (value - 1) / align
    }
}
