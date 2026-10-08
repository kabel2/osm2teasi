"""Audit the verified outer layout of a Falko/Tahuna chart file.

This deliberately stops before interpreting the high-entropy payload.  It
prevents mistaking the middle of the variable-length directory for its start.
"""

from __future__ import annotations

import argparse
import collections
import lzma
import struct
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Tile:
    x: int
    y: int
    start: int
    end: int
    blob: bytes


@dataclass(frozen=True)
class Record:
    tile: Tile
    relative_start: int
    length: int
    pltx: int
    first_byte: int


LZMA1_FILTER = {
    "id": lzma.FILTER_LZMA1,
    "dict_size": 0x01000000,
    "lc": 3,
    "lp": 0,
    "pb": 2,
}


def parse(path: Path) -> tuple[list[Tile], list[Record]]:
    data = path.read_bytes()
    tile_count = struct.unpack_from("<I", data, 0x74)[0]
    directory_end = 0x78 + 8 * tile_count
    if directory_end > len(data):
        raise ValueError("tile directory exceeds file")

    directory = [struct.unpack_from("<HHI", data, 0x78 + 8 * index) for index in range(tile_count)]
    if any(a[2] >= b[2] for a, b in zip(directory, directory[1:])):
        raise ValueError("tile offsets are not strictly increasing")

    tiles = [
        Tile(x, y, start, directory[index + 1][2] if index + 1 < tile_count else len(data), data[start + 0x157C : start + 0x159C])
        for index, (x, y, start) in enumerate(directory)
    ]
    records: list[Record] = []
    for tile in tiles:
        slot_starts = {
            struct.unpack_from("<I", data, tile.start + 0x140 + 4 * slot)[0]
            for slot in range((0x157C - 0x140) // 4)
        }
        relative_start = 0x159C
        while relative_start in slot_starts:
            start = tile.start + relative_start
            if start + 8 > tile.end:
                raise ValueError(f"record header beyond tile ({tile.x}, {tile.y})")
            length, pltx = struct.unpack_from("<II", data, start)
            if length == 0 or start + 8 + length > tile.end:
                raise ValueError(f"record body beyond tile ({tile.x}, {tile.y})")
            records.append(Record(tile, relative_start, length, pltx, data[start + 8]))
            relative_start += 8 + length
    return tiles, records


def derive_key(encrypted_blob: bytes, seed: bytes) -> bytes:
    """Port FUN_002897b0: decrypt a 32-byte tile key with a 32-byte seed."""
    if len(encrypted_blob) != 32 or len(seed) != 32:
        raise ValueError("both encrypted blob and seed must be exactly 32 bytes")

    state = bytearray(0x80)
    state[0x3E : 0x3E + 32] = seed
    output = bytearray(encrypted_blob)

    for offset in range(32):
        state[0x7E] = output[offset]
        _update_state(state)
        result = ((state[0x36] & 0xFF) ^ (state[0x36] >> 8) ^ state[0x7E]) & 0xFF
        state[0x7E] = result
        for key_offset in range(32):
            state[0x3E + key_offset] ^= result
        output[offset] = result

    return bytes(output)


def transform_payload(payload: bytes, key: bytes) -> bytes:
    """Port FUN_002898ec with a prepared 32-byte per-tile key."""
    if len(key) != 32:
        raise ValueError("the transform key must be exactly 32 bytes")

    state = bytearray(0x80)
    state[0x3E : 0x3E + 32] = key
    output = bytearray(payload)

    for offset in range(min(len(output), 100)):
        output[offset] = _transform_byte(output[offset], state)

    # The ARM implementation deliberately processes only every tenth byte after
    # its first 100-byte prefix; preserve that unusual wire-compatible behavior.
    for offset in range(100, len(output), 10):
        output[offset] = _transform_byte(output[offset], state)
    return bytes(output)


def _update_state(state: bytearray) -> None:
    """Port FUN_00289660 + FUN_002896c8: advance the 32-byte key state."""
    accumulator = state[0x36] | (state[0x37] << 8)
    previous = 0

    for index in range(16):
        key_base = 0x3E + 2 * index
        work_base = 0x12 + 2 * index
        word = (((state[key_base] << 8) | state[key_base + 1]) ^ previous) & 0xFFFF
        previous = word

        mixed = (word * 0x15A) & 0xFFFF
        lcg = (word * 0x4E35 + 1) & 0xFFFF
        old_c = state[0x0C] | (state[0x0D] << 8)
        old_10 = state[0x10] | (state[0x11] << 8)
        temp = ((index + old_10) * 0x4E35 + mixed) & 0xFFFF
        s4 = (old_c + temp) & 0xFFFF
        accumulator ^= s4 ^ lcg

        state[work_base] = lcg & 0xFF
        state[work_base + 1] = (lcg >> 8) & 0xFF
        state[0x0C] = mixed & 0xFF
        state[0x0D] = (mixed >> 8) & 0xFF
        state[0x0E] = mixed & 0xFF
        state[0x0F] = (mixed >> 8) & 0xFF
        state[0x08] = temp & 0xFF
        state[0x09] = (temp >> 8) & 0xFF
        state[0x0A] = s4 & 0xFF
        state[0x0B] = (s4 >> 8) & 0xFF
        state[0x10] = s4 & 0xFF
        state[0x11] = (s4 >> 8) & 0xFF

    state[0x32] = (s4 ^ lcg) & 0xFF
    state[0x33] = ((s4 ^ lcg) >> 8) & 0xFF
    state[0x36] = accumulator & 0xFF
    state[0x37] = (accumulator >> 8) & 0xFF
    state[0x34] = 0
    state[0x35] = 0


def _transform_byte(value: int, state: bytearray) -> int:
    state[0x7E] = value
    _update_state(state)
    result = ((state[0x36] & 0xFF) ^ (state[0x36] >> 8) ^ state[0x7E]) & 0xFF
    for key_offset in range(32):
        state[0x3E + key_offset] ^= result
    return result


def decompress_lzma1(payload: bytes, output_size: int) -> tuple[bool, int]:
    try:
        decoder = lzma.LZMADecompressor(format=lzma.FORMAT_RAW, filters=[LZMA1_FILTER])
        decoded = decoder.decompress(payload, max_length=output_size + 1)
    except lzma.LZMAError:
        return False, 0
    return decoder.eof and len(decoded) == output_size, len(decoded)


def probe_lzma(path: Path, records: list[Record]) -> None:
    data = path.read_bytes()
    direct_successes = 0
    best_seed: bytes | None = None
    best_count = 0

    # Candidate seeds for the global key context at +0x88.  The firmware default
    # path stores 0x10000 there when no external key table is present.
    candidate_seeds = [
        (0x00010000).to_bytes(4, "little") + bytes(28),
        bytes(32),
        (0x00010000).to_bytes(4, "little") * 8,
        (0x00010001).to_bytes(4, "little") + bytes(28),
        (0x00000001).to_bytes(4, "little") + bytes(28),
    ]

    results: dict[bytes, tuple[int, collections.Counter[int]]] = {}
    for seed in candidate_seeds:
        transformed_successes = 0
        transformed_sizes: collections.Counter[int] = collections.Counter()
        for record in records:
            payload_start = record.tile.start + record.relative_start + 8
            payload = data[payload_start : payload_start + record.length]
            key = derive_key(record.tile.blob, seed)
            transformed = transform_payload(payload, key)
            valid, decoded_size = decompress_lzma1(transformed, record.pltx)
            transformed_successes += valid
            transformed_sizes[decoded_size] += 1
        results[seed] = (transformed_successes, transformed_sizes)
        if transformed_successes > best_count:
            best_count = transformed_successes
            best_seed = seed

    print("raw LZMA1 probe (properties 5d 00 00 00 01):")
    print(f"  untransformed valid={direct_successes}/{len(records)}")
    for seed, (successes, sizes) in results.items():
        marker = " <-- best" if seed == best_seed else ""
        common_sizes = ", ".join(
            f"{size}:{count}" for size, count in sizes.most_common(5)
        )
        print(
            f"  seed={seed[:4].hex()} valid={successes}/{len(records)} "
            f"sizes={common_sizes}{marker}"
        )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("file", type=Path)
    parser.add_argument(
        "--probe-lzma",
        action="store_true",
        help="test raw LZMA1 before and after the firmware's tile-key transform",
    )
    args = parser.parse_args()
    tiles, records = parse(args.file)
    by_blob: dict[bytes, list[Record]] = collections.defaultdict(list)
    for record in records:
        by_blob[record.tile.blob].append(record)

    print(f"file={args.file}")
    print(f"tiles={len(tiles)} records={len(records)}")
    print(f"directory=0x78..0x{0x78 + 8 * len(tiles):x}; first-tile=0x{tiles[0].start:x}")
    print("groups by 32-byte blob:")
    for blob, group in sorted(by_blob.items(), key=lambda item: (item[0], item[1][0].tile.start)):
        first_bytes = {record.first_byte for record in group}
        print(
            f"  blob={blob[:4].hex()} tiles={len({record.tile.start for record in group})} "
            f"records={len(group)} first={'/'.join(f'{value:02x}' for value in sorted(first_bytes))}"
        )
    if args.probe_lzma:
        probe_lzma(args.file, records)


if __name__ == "__main__":
    main()
