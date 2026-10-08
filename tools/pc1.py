"""PC1 (Pukall Cipher 1, 256-bit) as used by bikenav.exe.

Verified against FUN_00289660 / FUN_002896c8 (keystream), FUN_002897b0
(full decrypt of the 32-B tile blob) and FUN_002898ec (partial decrypt of a
record payload: bytes 0..99, then every 10th byte).
"""


class PC1:
    def __init__(self, key: bytes):
        assert len(key) == 32
        self.cle = bytearray(key)
        self.si = 0
        self.x1a2 = 0
        self.x1a0 = [0] * 16

    def _assemble(self) -> int:
        cle, x1a0 = self.cle, self.x1a0
        inter = 0
        prev = 0
        si, x1a2 = self.si, self.x1a2
        for i in range(16):
            x1a0[i] = ((cle[2 * i] << 8) | cle[2 * i + 1]) ^ prev
            # code()
            dx = (x1a2 + i) & 0xFFFF
            ax = x1a0[i]
            cx = (dx * 0x4E35) & 0xFFFF
            cx = (cx + ax * 0x15A) & 0xFFFF
            dx = (cx + si) & 0xFFFF
            si = (ax * 0x15A) & 0xFFFF
            ax = (ax * 0x4E35 + 1) & 0xFFFF
            x1a2 = dx
            x1a0[i] = ax
            inter ^= ax ^ dx
            prev = ax
        self.si, self.x1a2 = si, x1a2
        return inter

    def dec_byte(self, c: int) -> int:
        inter = self._assemble()
        c ^= (inter >> 8) ^ (inter & 0xFF)
        for k in range(32):
            self.cle[k] ^= c
        return c

    def enc_byte(self, p: int) -> int:
        inter = self._assemble()
        for k in range(32):
            self.cle[k] ^= p
        return p ^ (inter >> 8) ^ (inter & 0xFF)


def decrypt_blob(blob: bytes, key: bytes) -> bytes:
    c = PC1(key)
    return bytes(c.dec_byte(b) for b in blob)


def encrypt_blob(blob: bytes, key: bytes) -> bytes:
    c = PC1(key)
    return bytes(c.enc_byte(b) for b in blob)


def _positions(n: int):
    yield from range(min(n, 100))
    yield from range(100, n, 10)


def decrypt_payload(data: bytes, key: bytes) -> bytes:
    out = bytearray(data)
    c = PC1(key)
    for p in _positions(len(out)):
        out[p] = c.dec_byte(out[p])
    return bytes(out)


def encrypt_payload(data: bytes, key: bytes) -> bytes:
    out = bytearray(data)
    c = PC1(key)
    for p in _positions(len(out)):
        out[p] = c.enc_byte(out[p])
    return bytes(out)
