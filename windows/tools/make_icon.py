#!/usr/bin/env python3
"""从 macOS 品牌图生成 Windows 图标资源。

用法（在 windows/ 目录下执行）：
    python tools/make_icon.py

输入：../Resources/AppIcon.png（1024x1024 RGBA 品牌图）
输出：
    resources/app.ico        —— 打包进 exe 的多尺寸图标（16~256）
    resources/app-64.png     —— 预览用
    src/app_icon.rs          —— 运行时窗口图标（64x64 RGBA 常量）

只依赖标准库（zlib/struct），无需 Pillow。
"""

from __future__ import annotations

import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)

SRC = os.path.join(ROOT, "..", "Resources", "AppIcon.png")
OUT_ICO = os.path.join(ROOT, "resources", "app.ico")
OUT_PREVIEW = os.path.join(ROOT, "resources", "app-64.png")
OUT_RUST = os.path.join(ROOT, "src", "app_icon.rs")

# Windows 图标里圆角方块留白比 macOS 小一些，让形状在任务栏更"满"
PADDING_RATIO = 0.035
SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
PNG_SIZES = {128, 256}  # 大尺寸用 PNG 压缩，避免 ICO 膨胀


# ---------------------------------------------------------------- PNG 解码

def _paeth(a: int, b: int, c: int) -> int:
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    return b if pb <= pc else c


def read_png(path: str):
    data = open(path, "rb").read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "不是 PNG"
    pos, idat = 8, b""
    w = h = 0
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        kind = data[pos + 4 : pos + 8]
        payload = data[pos + 8 : pos + 8 + length]
        if kind == b"IHDR":
            w, h, depth, color, _, _, interlace = struct.unpack(">IIBBBBB", payload)
            assert depth == 8 and color == 6 and interlace == 0, "只支持 8bit RGBA"
        elif kind == b"IDAT":
            idat += payload
        elif kind == b"IEND":
            break
        pos += 12 + length

    raw = zlib.decompress(idat)
    stride = w * 4
    out = bytearray(w * h * 4)
    prev = bytearray(stride)
    for y in range(h):
        base = y * (stride + 1)
        ftype = raw[base]
        line = bytearray(raw[base + 1 : base + 1 + stride])
        if ftype == 1:
            for i in range(4, stride):
                line[i] = (line[i] + line[i - 4]) & 0xFF
        elif ftype == 2:
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 0xFF
        elif ftype == 3:
            for i in range(stride):
                left = line[i - 4] if i >= 4 else 0
                line[i] = (line[i] + ((left + prev[i]) >> 1)) & 0xFF
        elif ftype == 4:
            for i in range(stride):
                left = line[i - 4] if i >= 4 else 0
                ul = prev[i - 4] if i >= 4 else 0
                line[i] = (line[i] + _paeth(left, prev[i], ul)) & 0xFF
        out[y * stride : (y + 1) * stride] = line
        prev = line
    return w, h, out


def png_bytes(w: int, h: int, rgba: bytes) -> bytes:
    raw = bytearray()
    stride = w * 4
    for y in range(h):
        raw.append(0)
        raw += rgba[y * stride : (y + 1) * stride]

    def chunk(kind: bytes, payload: bytes) -> bytes:
        return (
            struct.pack(">I", len(payload))
            + kind
            + payload
            + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)
        )

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def write_png(path: str, w: int, h: int, rgba: bytes) -> None:
    open(path, "wb").write(png_bytes(w, h, rgba))


# ---------------------------------------------------------------- 像素处理

def to_linear(c: int) -> float:
    v = c / 255.0
    return v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4


def to_srgb(v: float) -> int:
    v = min(max(v, 0.0), 1.0)
    s = v * 12.92 if v <= 0.0031308 else 1.055 * (v ** (1 / 2.4)) - 0.055
    return int(round(s * 255))


LUT = [to_linear(i) for i in range(256)]


def alpha_bbox(w: int, h: int, rgba: bytearray):
    minx, miny, maxx, maxy = w, h, -1, -1
    for y in range(h):
        row = y * w * 4
        for x in range(w):
            if rgba[row + x * 4 + 3] > 8:
                if x < minx:
                    minx = x
                if x > maxx:
                    maxx = x
                if y < miny:
                    miny = y
                if y > maxy:
                    maxy = y
    return minx, miny, maxx + 1, maxy + 1


def premultiply_linear(w: int, h: int, rgba: bytearray):
    """返回 (R,G,B,A) 四个按行存储的 float 列表，颜色为线性光、已预乘 alpha。"""
    n = w * h
    pr = [0.0] * n
    pg = [0.0] * n
    pb = [0.0] * n
    pa = [0.0] * n
    for i in range(n):
        a = rgba[i * 4 + 3]
        if a == 0:
            continue
        af = a / 255.0
        pr[i] = LUT[rgba[i * 4]] * af
        pg[i] = LUT[rgba[i * 4 + 1]] * af
        pb[i] = LUT[rgba[i * 4 + 2]] * af
        pa[i] = af
    return pr, pg, pb, pa


def resample(src, sw: int, sh: int, dw: int, dh: int):
    """精确面积重采样（预乘线性光），返回 float 四元组。"""
    pr, pg, pb, pa = src
    scale_x = sw / dw
    scale_y = sh / dh

    # 预计算水平方向权重
    cols = []
    for dx in range(dw):
        x0, x1 = dx * scale_x, (dx + 1) * scale_x
        weights = []
        for sx in range(int(x0), min(int(x1) + 1, sw)):
            cover = min(x1, sx + 1) - max(x0, sx)
            if cover > 0:
                weights.append((sx, cover / scale_x))
        cols.append(weights)

    rows = []
    for dy in range(dh):
        y0, y1 = dy * scale_y, (dy + 1) * scale_y
        weights = []
        for sy in range(int(y0), min(int(y1) + 1, sh)):
            cover = min(y1, sy + 1) - max(y0, sy)
            if cover > 0:
                weights.append((sy, cover / scale_y))
        rows.append(weights)

    n = dw * dh
    orr = [0.0] * n
    org = [0.0] * n
    orb = [0.0] * n
    oa = [0.0] * n
    for dy, rweights in enumerate(rows):
        for dx, cweights in enumerate(cols):
            r = g = b = a = 0.0
            for sy, wy in rweights:
                base = sy * sw
                for sx, wx in cweights:
                    idx = base + sx
                    wgt = wx * wy
                    r += pr[idx] * wgt
                    g += pg[idx] * wgt
                    b += pb[idx] * wgt
                    a += pa[idx] * wgt
            k = dy * dw + dx
            orr[k], org[k], orb[k], oa[k] = r, g, b, a
    return orr, org, orb, oa


def to_rgba(src, w: int, h: int) -> bytes:
    pr, pg, pb, pa = src
    out = bytearray(w * h * 4)
    for i in range(w * h):
        a = pa[i]
        if a <= 0.0001:
            continue
        out[i * 4] = to_srgb(pr[i] / a)
        out[i * 4 + 1] = to_srgb(pg[i] / a)
        out[i * 4 + 2] = to_srgb(pb[i] / a)
        out[i * 4 + 3] = int(round(min(a, 1.0) * 255))
    return bytes(out)


def crop(src, sw: int, sh: int, box, pad_ratio: float, target: int):
    """把 box 区域按 pad 居中缩放到 target×target 的方图里。"""
    x0, y0, x1, y1 = box
    bw, bh = x1 - x0, y1 - y0
    side = max(bw, bh)
    # 先裁成正方形（居中），再整体缩放
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    sx0, sy0 = int(round(cx - side / 2)), int(round(cy - side / 2))
    sx0 = max(sx0, 0)
    sy0 = max(sy0, 0)

    pr, pg, pb, pa = src
    square = ([0.0] * (side * side), [0.0] * (side * side),
              [0.0] * (side * side), [0.0] * (side * side))
    for y in range(side):
        sy = sy0 + y
        if sy >= sh:
            break
        for x in range(side):
            sx = sx0 + x
            if sx >= sw:
                break
            s = sy * sw + sx
            d = y * side + x
            square[0][d] = pr[s]
            square[1][d] = pg[s]
            square[2][d] = pb[s]
            square[3][d] = pa[s]

    inner = int(round(target * (1 - 2 * pad_ratio)))
    shrunk = resample(square, side, side, inner, inner)

    # 居中贴到透明画布
    out = ([0.0] * (target * target), [0.0] * (target * target),
           [0.0] * (target * target), [0.0] * (target * target))
    off = (target - inner) // 2
    for y in range(inner):
        for x in range(inner):
            s = y * inner + x
            d = (y + off) * target + (x + off)
            out[0][d] = shrunk[0][s]
            out[1][d] = shrunk[1][s]
            out[2][d] = shrunk[2][s]
            out[3][d] = shrunk[3][s]
    return to_rgba(out, target, target)


# ---------------------------------------------------------------- ICO 封装

def dib_entry(size: int, rgba: bytes) -> bytes:
    header = struct.pack(
        "<IiiHHIIiiII",
        40,          # biSize
        size,        # biWidth
        size * 2,    # biHeight（颜色 + 掩码）
        1, 32, 0,    # biPlanes, biBitCount, biCompression
        size * size * 4,
        0, 0, 0, 0,
    )
    pixels = bytearray()
    for y in range(size - 1, -1, -1):
        row = y * size * 4
        for x in range(size):
            r, g, b, a = rgba[row + x * 4 : row + x * 4 + 4]
            pixels += bytes((b, g, r, a))
    mask_row = ((size + 31) // 32) * 4
    mask = bytes(mask_row * size)
    return header + bytes(pixels) + mask


def build_ico(images: dict[int, bytes], png_sizes: set[int]) -> bytes:
    sizes = sorted(images)
    entries = []
    blobs = []
    offset = 6 + 16 * len(sizes)
    for size in sizes:
        if size in png_sizes:
            blob = png_bytes(size, size, images[size])
        else:
            blob = dib_entry(size, images[size])
        entries.append((size, len(blob), offset))
        blobs.append(blob)
        offset += len(blob)

    head = struct.pack("<HHH", 0, 1, len(sizes))
    directory = b"".join(
        struct.pack("<BBBBHHII", s if s < 256 else 0, s if s < 256 else 0,
                    0, 0, 1, 32, length, off)
        for s, length, off in entries
    )
    return head + directory + b"".join(blobs)


# ---------------------------------------------------------------- 主流程

def main() -> None:
    w, h, rgba = read_png(SRC)
    box = alpha_bbox(w, h, rgba)
    print("源图", w, h, "内容区域", box)

    src = premultiply_linear(w, h, rgba)
    # 先降到一个中等尺寸再逐级缩放，既快又稳
    base = resample(src, w, h, 512, 512)
    base_rgba = to_rgba(base, 512, 512)
    base_box = alpha_bbox(512, 512, bytearray(base_rgba))
    base_src = premultiply_linear(512, 512, bytearray(base_rgba))

    images = {}
    for size in SIZES:
        images[size] = crop(base_src, 512, 512, base_box, PADDING_RATIO, size)
        print(f"  {size}x{size} ok")

    ico = build_ico(images, PNG_SIZES)
    open(OUT_ICO, "wb").write(ico)
    print("写入", OUT_ICO, len(ico), "字节")

    write_png(OUT_PREVIEW, 64, 64, images[64])

    # 运行时窗口图标：64x64 RGBA
    small = images[64]
    body = ",".join(str(b) for b in small)
    rust = (
        "// 由 windows/tools/make_icon.py 生成，请勿手工编辑。\n"
        "// 64x64 RGBA，用于设置窗口/任务栏图标。\n\n"
        "pub const SIDE: u32 = 64;\n\n"
        f"pub const RGBA: [u8; {len(small)}] = [{body}];\n"
    )
    open(OUT_RUST, "w", encoding="utf-8", newline="\n").write(rust)
    print("写入", OUT_RUST)


if __name__ == "__main__":
    main()
