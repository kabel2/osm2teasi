//! PC1 (Pukall Cipher 1, 256 bit) as used by bikenav.exe.
//!
//! Port of tools/pc1.py, which is verified against FUN_00289660 / FUN_002896c8
//! (keystream), FUN_002897b0 (full decrypt of the 32-byte tile blob) and
//! FUN_002898ec (partial decrypt of a record payload: bytes 0..99, then every
//! 10th byte).  All arithmetic is 16 bit and wraps.

pub struct Pc1 {
    cle: [u8; 32],
    si: u16,
    x1a2: u16,
    x1a0: [u16; 16],
}

impl Pc1 {
    pub fn new(key: &[u8]) -> Self {
        assert_eq!(key.len(), 32, "PC1 key must be 32 bytes");
        let mut cle = [0u8; 32];
        cle.copy_from_slice(key);
        Pc1 { cle, si: 0, x1a2: 0, x1a0: [0; 16] }
    }

    fn assemble(&mut self) -> u16 {
        let mut inter = 0u16;
        let mut prev = 0u16;
        for i in 0..16 {
            let mut ax = (u16::from(self.cle[2 * i]) << 8 | u16::from(self.cle[2 * i + 1])) ^ prev;
            self.x1a0[i] = ax;
            let dx = self.x1a2.wrapping_add(i as u16);
            let cx = dx.wrapping_mul(0x4E35).wrapping_add(ax.wrapping_mul(0x15A));
            let dx = cx.wrapping_add(self.si);
            self.si = ax.wrapping_mul(0x15A);
            ax = ax.wrapping_mul(0x4E35).wrapping_add(1);
            self.x1a2 = dx;
            self.x1a0[i] = ax;
            inter ^= ax ^ dx;
            prev = ax;
        }
        inter
    }

    fn mask(&mut self) -> u8 {
        let inter = self.assemble();
        (inter >> 8) as u8 ^ (inter & 0xFF) as u8
    }

    pub fn dec_byte(&mut self, c: u8) -> u8 {
        let p = c ^ self.mask();
        for b in self.cle.iter_mut() {
            *b ^= p;
        }
        p
    }

    pub fn enc_byte(&mut self, p: u8) -> u8 {
        let m = self.mask();
        for b in self.cle.iter_mut() {
            *b ^= p;
        }
        p ^ m
    }
}

/// Every byte (used for the 32-byte tile blob).
pub fn decrypt_blob(blob: &[u8], key: &[u8]) -> Vec<u8> {
    let mut c = Pc1::new(key);
    blob.iter().map(|&b| c.dec_byte(b)).collect()
}

pub fn encrypt_blob(blob: &[u8], key: &[u8]) -> Vec<u8> {
    let mut c = Pc1::new(key);
    blob.iter().map(|&b| c.enc_byte(b)).collect()
}

/// Byte positions the firmware enciphers in a record payload.
fn positions(n: usize) -> impl Iterator<Item = usize> {
    (0..n.min(100)).chain((100..n).step_by(10))
}

pub fn decrypt_payload(data: &[u8], key: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    let mut c = Pc1::new(key);
    for p in positions(out.len()) {
        out[p] = c.dec_byte(out[p]);
    }
    out
}

pub fn encrypt_payload(data: &[u8], key: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    let mut c = Pc1::new(key);
    for p in positions(out.len()) {
        out[p] = c.enc_byte(out[p]);
    }
    out
}
