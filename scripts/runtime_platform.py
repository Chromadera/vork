"""Qualify the OS floor of the complete native runtime, including companions."""
import re
import struct

FLOORS = {'linux-x64': (2, 35), 'darwin-arm64': (26, 0)}


# FAT headers have a 32- or 64-bit slice directory in either byte order.
FAT_FORMATS = {
    b'\xca\xfe\xba\xbe': ('>', 'IIIII'),
    b'\xbe\xba\xfe\xca': ('<', 'IIIII'),
    b'\xca\xfe\xba\xbf': ('>', 'IIQQII'),
    b'\xbf\xba\xfe\xca': ('<', 'IIQQII'),
}
MACH_MAGICS = {b'\xce\xfa\xed\xfe', b'\xfe\xed\xfa\xce',
               b'\xcf\xfa\xed\xfe', b'\xfe\xed\xfa\xcf'}


def _macho_floor(data, source, *, is_slice=False):
    magic = data[:4]
    required = (11, 0)
    if magic in FAT_FORMATS:
        if is_slice:
            raise ValueError(f'nested universal Mach-O slice: {source}')
        endian, fields = FAT_FORMATS[magic]
        entry_size = struct.calcsize(endian + fields)
        if len(data) < 8:
            raise ValueError(f'truncated universal Mach-O header: {source}')
        count = struct.unpack_from(endian + 'I', data, 4)[0]
        table_end = 8 + count * entry_size
        if not count or table_end > len(data):
            raise ValueError(f'invalid universal Mach-O slice directory: {source}')
        for index in range(count):
            entry = struct.unpack_from(endian + fields, data, 8 + index * entry_size)
            offset, size = entry[2:4]
            if offset < table_end or size < 4 or offset + size > len(data):
                raise ValueError(f'invalid universal Mach-O slice {index}: {source}')
            required = max(required, _macho_floor(data[offset:offset + size], source, is_slice=True))
        return required
    if magic != b'\xcf\xfa\xed\xfe':
        if is_slice or magic in MACH_MAGICS:
            raise ValueError(f'unsupported Mach-O slice: {source}; use a little-endian 64-bit runtime')
        return required  # Nonbinary resources do not impose a deployment target.
    if len(data) < 32:
        raise ValueError(f'truncated Mach-O header: {source}')
    offset = 32
    for _ in range(struct.unpack_from('<I', data, 16)[0]):
        if offset + 8 > len(data):
            raise ValueError(f'truncated Mach-O load command: {source}')
        command, size = struct.unpack_from('<II', data, offset)
        if size < 8 or offset + size > len(data):
            raise ValueError(f'invalid Mach-O load command: {source}')
        if command in (0x32, 0x24):
            field = 12 if command == 0x32 else 8
            if size < field + 4:
                raise ValueError(f'truncated Mach-O deployment target: {source}')
            value = struct.unpack_from('<I', data, offset + field)[0]
            required = max(required, (value >> 16, (value >> 8) & 255))
        offset += size
    return required


def check_runtime_platform(files, platform):
    minimum = (11, 0) if platform == 'darwin-arm64' else (2, 0)
    for source in files:
        data = source.read_bytes()
        required = minimum
        if platform == 'linux-x64' and data[:4] == b'\x7fELF':
            versions = [tuple(map(int, v)) for v in re.findall(rb'GLIBC_(\d+)\.(\d+)', data)]
            required = max(versions, default=minimum)
        elif platform == 'darwin-arm64':
            required = _macho_floor(data, source)
        if platform in FLOORS and required > FLOORS[platform]:
            label = 'glibc' if platform == 'linux-x64' else 'macOS'
            expected = '.'.join(map(str, FLOORS[platform]))
            actual = '.'.join(map(str, required))
            raise ValueError(f'{source} requires {label} {actual}, exceeding the {expected} package baseline')
        minimum = max(minimum, required)
    return minimum
