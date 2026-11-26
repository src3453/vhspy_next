"""Compare the URP PlayMode test output with the wgpu reference replay.

usage: python compare_urp.py <VHS_URP_OUT folder>

Expects raw.bin / gpu_raster.bin / presented.bin (+ .txt sizes) from VhsUrpTest and
wgpu_raster.bin from `cargo run -p vhs-slang --example replay`. Prints the raster difference,
the side-bar level and an orientation check, and writes PNGs (rows in texture memory order).
"""
import sys, numpy as np, zlib, struct
def png(path, img):
    img = (np.clip(img, 0, 1) ** (1/2.2) * 255 + 0.5).astype(np.uint8)
    h, w, _ = img.shape
    raw = b''.join(b'\x00' + img[y].tobytes() for y in range(h))
    def chunk(t, d): return struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    open(path, 'wb').write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b''))
def load(d, name):
    w, h = map(int, open(f'{d}/{name}.txt').read().split())
    return np.fromfile(f'{d}/{name}.bin', np.float32).reshape(h, w, 4)[..., :3]
d = sys.argv[1]
raw, gpu, pres = load(d, 'raw'), load(d, 'gpu_raster'), load(d, 'presented')
wg = np.fromfile(f'{d}/wgpu_raster.bin', np.float32).reshape(gpu.shape[0], gpu.shape[1], 4)[..., :3]
disp = lambda x: np.clip(x, 0, 1) ** (1/2.2) * 255
diff = np.abs(disp(gpu) - disp(wg))
print('unity raster vs wgpu replay: max %.2f mean %.3f LSB' % (diff.max(), diff.mean()))
# presented: 4:3 aperture at the centre of 1280x720 = columns 160..1120, black bars outside
bars = np.concatenate([pres[:, :150], pres[:, 1130:]], axis=1)
print('bars max value %.4f' % bars.max())
core_p, core_r = disp(pres[:, 170:1110]), disp(raw[:, 170:1110])
print('presented vs raw (same orientation) mean %.2f, vs vertically flipped raw mean %.2f' % (np.abs(core_p - core_r).mean(), np.abs(core_p - core_r[::-1]).mean()))
# which memory row holds the red marker? (top of scene)
red = (raw[..., 0] > 0.5) & (raw[..., 1] < 0.1) & (raw[..., 2] < 0.1)
rows = np.where(red.any(axis=1))[0]
print('red marker memory rows in raw: %d..%d of %d' % (rows.min(), rows.max(), raw.shape[0]))
png(f'{d}/raw.png', raw); png(f'{d}/presented.png', pres); png(f'{d}/gpu_raster.png', gpu)
