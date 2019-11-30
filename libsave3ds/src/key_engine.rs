/// Emulates 3DS AES key scrambler engine.

fn lrot128(a: [u8; 16], rot: usize) -> [u8; 16] {
    let mut out = [0; 16];
    let byte_shift = rot / 8;
    let bit_shift = rot % 8;
    for (i, o) in out.iter_mut().enumerate() {
        let wrap_index_a = (i + byte_shift) % 16;
        let wrap_index_b = (i + byte_shift + 1) % 16;
        // note: the right shift would be UB for bit_shift = 0.
        // good thing is that the values we will use for rot won't cause this
        *o = (a[wrap_index_a] << bit_shift) | (a[wrap_index_b] >> (8 - bit_shift));
    }
    out
}

fn add128(a: [u8; 16], b: [u8; 16]) -> [u8; 16] {
    let mut out = [0; 16];
    let mut carry = 0;

    for i in (0..16).rev() {
        let sum = u32::from(a[i]) + u32::from(b[i]) + carry;
        carry = sum >> 8;
        out[i] = (sum & 0xFF) as u8;
    }
    out
}

fn xor128(a: [u8; 16], b: [u8; 16]) -> [u8; 16] {
    let mut out = [0; 16];
    for i in 0..16 {
        out[i] = a[i] ^ b[i];
    }
    out
}

const SCRAMBLER: [u8; 16] = [
    0x1F, 0xF9, 0xE9, 0xAA, 0xC5, 0xFE, 0x04, 0x08, 0x02, 0x45, 0x91, 0xDC, 0x5D, 0x52, 0x76, 0x8A,
];

pub fn scramble(x: [u8; 16], y: [u8; 16]) -> [u8; 16] {
    lrot128(add128(xor128(lrot128(x, 2), y), SCRAMBLER), 87)
}
