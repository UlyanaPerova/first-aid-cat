#!/usr/bin/env python3
# Эталон для регрессионных тестов сценки плашки (engine/src/regress/scene.rs).
#
# Считает контрольные суммы КАДРОВ (436x130, FNV-1a 64 по байтам R,G,B каждой точки подряд) независимо от Rust-кода:
#   * фон и слои кота «спит / пишет / закончил» разжимаются здесь же, на Python, прямо из текста engine/src/scene_px.rs;
#   * кадр анимации = картинка состояния + точки утверждённых картинок владелицы (PNG из art/previews/..., их нет в git):
#       кардиограмма, огурчик, моргание, «z» — накладываются как есть; хвост — как в рабочем скрипте to_rust.py
#       (общий фон -> точки позы -> слой кота без точек «кот_стереть_*»).
# Результат — файл engine/src/regress/scene_expected.rs (константы). Запуск (нужен Python с PIL):
#   python3 tools/regress_scene_expected.py [путь/к/art/previews]
import os, re, sys
from PIL import Image

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
prev = sys.argv[1] if len(sys.argv) > 1 else os.path.join(root, 'art', 'previews')
A = os.path.join(prev, '2026-11-15_анимация_сценки', 'рабочие_файлы')
T = os.path.join(prev, '2026-11-15_хвост_по_методике', 'рабочие_файлы', 'final')
src = open(os.path.join(root, 'engine/src/scene_px.rs'), encoding='utf-8').read()

W = int(re.search(r'pub const W: i32 = (\d+)', src)[1]); H = int(re.search(r'pub const H: i32 = (\d+)', src)[1])
PAL = [int(x, 16) for x in re.search(r'PAL: \[u32; \d+\] = \[([^\]]*)\]', src)[1].replace('0x', '').replace(',', ' ').split()]

def unrle(s, n):
    # пары «символ значения, символ длины»: значение = код - 48, длина = код - 48 + 1; всё, что меньше '0', — переводы строк
    chars = [c for c in s if ord(c) >= 48]
    out = []
    for i in range(0, len(chars) - 1, 2):
        out += [ord(chars[i]) - 48] * (ord(chars[i + 1]) - 48 + 1)
    assert len(out) == n, (len(out), n)
    return out

BG = unrle(re.search(r'pub const BG: &str = r"(.*?)";', src, re.S)[1], W * H)
LAYERS = {}
for name, st in (('SLEEP', 0), ('WRITE', 1), ('DONE', 2)):
    m = re.search(rf'pub const {name}: Layer = \(\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*r"(.*?)",?\s*\);', src, re.S)
    x, y, w, h = map(int, m.groups()[:4])
    LAYERS[st] = (x, y, w, h, unrle(m[5], w * h))

def still(st):
    px = [PAL[i] for i in BG]
    x0, y0, w, h, v = LAYERS[st]
    for i, c in enumerate(v):
        if c:
            px[(y0 + i // w) * W + x0 + i % w] = PAL[c - 1]
    return px

def opaque(path):
    im = Image.open(path).convert('RGBA')
    assert im.size == (W, H), path
    px = im.load(); d = {}
    for y in range(H):
        for x in range(W):
            r, g, b, a = px[x, y]
            if a:
                assert a == 255, (path, x, y, a)
                d[(x, y)] = r << 16 | g << 8 | b
    return d

def fnv(px):
    h = 0xcbf29ce484222325
    for p in px:
        for b in ((p >> 16) & 255, (p >> 8) & 255, p & 255):
            h = ((h ^ b) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return h

def overlay(base, cells):
    px = list(base)
    for (x, y), c in cells.items():
        px[y * W + x] = c
    return px

def tail_cells(st, pose):
    # как tail() в to_rust.py: общий фон -> точки позы -> слой кота без «стереть»; итог — картинка целиком
    px = [PAL[i] for i in BG]
    for (x, y), c in opaque(f'{T}/ALL_хвост_{pose}.png').items():
        px[y * W + x] = c
    skip = set(opaque(f'{T}/кот_стереть_{("SLEEP", "WRITE", "DONE")[st]}.png'))
    x0, y0, w, h, v = LAYERS[st]
    for i, c in enumerate(v):
        x, y = x0 + i % w, y0 + i // w
        if c and (x, y) not in skip:
            px[y * W + x] = PAL[c - 1]
    return px

F = os.path.join(A, 'final')
Z = os.path.join(A, 'отклонено_1', 'patches')
cases = []  # (имя, хеш)
for st in range(3):
    cases.append((f'still_{st}', fnv(still(st))))
for st in range(3):
    s = still(st)
    for i in range(20):
        cases.append((f'ecg_{st}_{i}', fnv(overlay(s, opaque(f'{F}/ALL_кардио_{i:02}.png')))))
    for k, n in ((1, 'вверх'), (2, 'вниз')):
        cases.append((f'pickle_{st}_{k}', fnv(overlay(s, opaque(f'{F}/ALL_огурец_{n}.png')))))
    for pose in (1, 2, 3):
        cases.append((f'tail_{st}_{pose}', fnv(tail_cells(st, f'P{pose}'))))
for st, nm in ((1, 'WRITE'), (2, 'DONE')):
    s = still(st)
    for k, n in ((1, 'полу'), (2, 'закрыты')):
        cases.append((f'blink_{st}_{k}', fnv(overlay(s, opaque(f'{F}/{nm}_моргание_{n}.png')))))
for k in (1, 2):
    cases.append((f'z_0_{k}', fnv(overlay(still(0), opaque(f'{Z}/SLEEP_z_{k}.png')))))

# составные кадры: порядок наложения — хвост, кардиограмма, огурчик, «z», веки (как в overlays())
def combo(st, tail=0, ecg=None, pickle=0, z=0, blink=0):
    px = still(st)
    if tail:
        px = tail_cells(st, f'P{tail}')
    if ecg is not None:
        px = overlay(px, opaque(f'{F}/ALL_кардио_{ecg:02}.png'))
    if pickle:
        px = overlay(px, opaque(f'{F}/ALL_огурец_{("вверх", "вниз")[pickle - 1]}.png'))
    if z:
        px = overlay(px, opaque(f'{Z}/SLEEP_z_{z}.png'))
    if blink:
        px = overlay(px, opaque(f'{F}/{("WRITE", "DONE")[st - 1]}_моргание_{("полу", "закрыты")[blink - 1]}.png'))
    return fnv(px)

COMBOS = [
    ('combo_a', 1, dict(tail=2, ecg=5, pickle=1, blink=2)),
    ('combo_b', 0, dict(tail=3, ecg=11, pickle=2, z=2)),
    ('combo_c', 2, dict(tail=1, ecg=0, pickle=1, blink=1)),
    ('combo_d', 1, dict(tail=3, ecg=19, pickle=2, blink=1)),
]
for name, st, kw in COMBOS:
    cases.append((name, combo(st, **kw)))

out = ['// Эталон кадров сценки: контрольные суммы (FNV-1a 64 по байтам R,G,B точек кадра 436x130 подряд).',
       '// Сгенерировано tools/regress_scene_expected.py из утверждённых картинок владелицы (art/previews/2026-11-15_*,',
       '// в репозиторий не входят) и из текста scene_px.rs, разжатого на Python. Руками не править.',
       f'pub const CASES: [(&str, u64); {len(cases)}] = [']
out += [f'    ("{n}", 0x{h:016x}),' for n, h in cases]
out.append('];')
open(os.path.join(root, 'engine/src/regress/scene_expected.rs'), 'w', encoding='utf-8').write('\n'.join(out) + '\n')
print('готово:', len(cases), 'кадров')
