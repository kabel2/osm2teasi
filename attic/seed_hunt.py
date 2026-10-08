#!/usr/bin/env python3
"""Brute-force search for the global seed using real ARM emulation via Unicorn.

This avoids any porting mistakes by executing the firmware's actual
FUN_00289660/FUN_002896c8 update routine for every state advance.
"""
from __future__ import annotations

import collections
import lzma
import struct
import sys
from pathlib import Path

from unicorn import Uc, UcError, UC_ARCH_ARM, UC_MODE_ARM
from unicorn.arm_const import UC_ARM_REG_R0, UC_ARM_REG_SP


# Globals initialised by init_emu()
_mu: Uc | None = None
_state_addr = 0x60000000
_stack_top = 0x70000000
_update_start = 0x00289660
_update_end = 0x002896c4


def init_emu(exe_path: Path) -> None:
    global _mu
    data = exe_path.read_bytes()
    pe_offset = struct.unpack_from("<I", data, 0x3C)[0]
    base = struct.unpack_from("<I", data, pe_offset + 0x34)[0]
    num_sections = struct.unpack_from("<H", data, pe_offset + 0x6)[0]
    sections = []
    for i in range(num_sections):
        sec_off = pe_offset + 0xF8 + 40 * i
        vsize = struct.unpack_from("<I", data, sec_off + 0x8)[0]
        vaddr = struct.unpack_from("<I", data, sec_off + 0xC)[0]
        raw_size = struct.unpack_from("<I", data, sec_off + 0x10)[0]
        raw_addr = struct.unpack_from("<I", data, sec_off + 0x14)[0]
        sections.append((vaddr, vsize, raw_addr, raw_size))

    vaddr, vsize, raw_addr, raw_size = sections[0]
    text_va = base + vaddr
    text_data = data[raw_addr : raw_addr + raw_size]

    mu = Uc(UC_ARCH_ARM, UC_MODE_ARM)
    mu.mem_map(text_va, (len(text_data) + 0xFFF) & ~0xFFF)
    mu.mem_write(text_va, text_data)

    mu.mem_map(_stack_top - 0x10000, 0x10000)
    mu.mem_map(_state_addr, 0x1000)

    _mu = mu


def emu_update_state(state: bytearray) -> bytearray:
    assert _mu is not None
    _mu.mem_write(_state_addr, bytes(state))
    _mu.reg_write(UC_ARM_REG_R0, _state_addr)
    _mu.reg_write(UC_ARM_REG_SP, _stack_top - 0x100)
    _mu.emu_start(_update_start, _update_end)
    return bytearray(_mu.mem_read(_state_addr, 0x80))


def derive_key(encrypted_blob: bytes, seed: bytes) -> bytes:
    if len(encrypted_blob) != 32 or len(seed) != 32:
        raise ValueError("both encrypted blob and seed must be exactly 32 bytes")

    state = bytearray(0x80)
    state[0x3E : 0x3E + 32] = seed
    output = bytearray(encrypted_blob)

    for offset in range(32):
        state[0x7E] = output[offset]
        state[0x7F] = 0
        state = emu_update_state(state)
        acc = state[0x36] | (state[0x37] << 8)
        result = ((acc & 0xFF) ^ (acc >> 8) ^ (state[0x7E] | (state[0x7F] << 8))) & 0xFF
        state[0x7E] = result
        state[0x7F] = 0
        for key_offset in range(32):
            state[0x3E + key_offset] ^= result
        output[offset] = result

    return bytes(output)


def transform_payload(payload: bytes, key: bytes) -> bytes:
    if len(key) != 32:
        raise ValueError("the transform key must be exactly 32 bytes")

    state = bytearray(0x80)
    state[0x3E : 0x3E + 32] = key
    output = bytearray(payload)

    for offset in range(min(len(output), 100)):
        output[offset] = _transform_byte(output[offset], state)

    for offset in range(100, len(output), 10):
        output[offset] = _transform_byte(output[offset], state)
    return bytes(output)


def _transform_byte(value: int, state: bytearray) -> int:
    state[0x7E] = value
    state[0x7F] = 0
    state[:] = emu_update_state(state)
    acc = state[0x36] | (state[0x37] << 8)
    result = ((acc & 0xFF) ^ (acc >> 8) ^ (state[0x7E] | (state[0x7F] << 8))) & 0xFF
    for key_offset in range(32):
        state[0x3E + key_offset] ^= result
    return result


LZMA1_FILTER = {
    "id": lzma.FILTER_LZMA1,
    "dict_size": 0x01000000,
    "lc": 3,
    "lp": 0,
    "pb": 2,
}


def decompress_lzma1(payload: bytes, output_size: int) -> tuple[bool, int]:
    try:
        decoder = lzma.LZMADecompressor(format=lzma.FORMAT_RAW, filters=[LZMA1_FILTER])
        decoded = decoder.decompress(payload, max_length=output_size + 1)
    except lzma.LZMAError:
        return False, 0
    return decoder.eof and len(decoded) == output_size, len(decoded)


def parse(path: Path) -> tuple[list[dict], list[dict]]:
    data = path.read_bytes()
    tile_count = struct.unpack_from("<I", data, 0x74)[0]
    directory = [
        struct.unpack_from("<HHI", data, 0x78 + 8 * index) for index in range(tile_count)
    ]
    tiles = []
    for index, (x, y, start) in enumerate(directory):
        end = directory[index + 1][2] if index + 1 < tile_count else len(data)
        tiles.append(
            {
                "x": x,
                "y": y,
                "start": start,
                "end": end,
                "blob": data[start + 0x157C : start + 0x159C],
            }
        )

    records = []
    for tile in tiles:
        slot_starts = {
            struct.unpack_from("<I", data, tile["start"] + 0x140 + 4 * slot)[0]
            for slot in range((0x157C - 0x140) // 4)
        }
        relative_start = 0x159C
        while relative_start in slot_starts:
            start = tile["start"] + relative_start
            if start + 8 > tile["end"]:
                break
            length, pltx = struct.unpack_from("<II", data, start)
            if length == 0 or start + 8 + length > tile["end"]:
                break
            records.append(
                {
                    "tile": tile,
                    "relative_start": relative_start,
                    "length": length,
                    "pltx": pltx,
                }
            )
            relative_start += 8 + length
    return tiles, records


def check_seed(seed: bytes, records: list[dict], data: bytes) -> int:
    successes = 0
    for record in records:
        payload_start = record["tile"]["start"] + record["relative_start"] + 8
        payload = data[payload_start : payload_start + record["length"]]
        key = derive_key(record["tile"]["blob"], seed)
        transformed = transform_payload(payload, key)
        valid, _ = decompress_lzma1(transformed, record["pltx"])
        if valid:
            successes += 1
    return successes


def main() -> int:
    if len(sys.argv) != 3:
        print(f"usage: {sys.argv[0]} <bikenav.exe> <chart-file>")
        return 1

    exe_path = Path(sys.argv[1])
    chart_path = Path(sys.argv[2])

    print("Initialising Unicorn...")
    init_emu(exe_path)

    tiles, records = parse(chart_path)
    data = chart_path.read_bytes()
    print(f"{chart_path}: tiles={len(tiles)} records={len(records)}")

    # Test a small set of obvious seeds first.
    candidates = [
        bytes(32),
        bytes([0xFF] * 32),
        b"Tahuna" + bytes(26),
        b"Falko" + bytes(27),
        b"osmpoint" + bytes(24),
        b"bikenav" + bytes(25),
    ]
    for value in [0x00010000, 0x12345678, 0xDEADBEEF, 0x55AA55AA, 0x00000001]:
        candidates.append(value.to_bytes(4, "little") * 8)
        candidates.append(value.to_bytes(4, "big") * 8)

    best = 0
    best_seed: bytes | None = None
    for seed in candidates:
        count = check_seed(seed, records, data)
        print(f"seed={seed[:4].hex()}... valid={count}/{len(records)}")
        if count > best:
            best = count
            best_seed = seed

    print(f"\nbest seed={best_seed.hex() if best_seed else None} valid={best}/{len(records)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
