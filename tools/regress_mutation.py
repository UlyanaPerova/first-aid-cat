#!/usr/bin/env python3
"""Проверка самих тестов: в КОПИИ исходников (рабочее дерево не трогается) по одной вносится заведомая ошибка в проверяемую
функцию, и регрессионные тесты ОБЯЗАНЫ упасть. Если тест не упал — он не способен ловить такую ошибку.

Запуск: tools/regress_mutation.py [--keep] [номера мутаций через запятую]
Копия — /tmp/aptechka_mut (внутри ссылки на assets/, packaging/, release/, art/layers репозитория, они только читаются).
Сборка ускорена (без LTO): CARGO_PROFILE_RELEASE_LTO=false, codegen-units=16, opt-level=1.
"""
import os, shutil, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
M = '/tmp/aptechka_mut'
TARGET = '/tmp/aptechka_mut_target'

# (файл, было, стало, что должно упасть (подстрока имени теста), смысл)
MUTS = [
    ('diag.rs', 'd[(d.len() - 1) / 2]', 'd[d.len() - 1]', 'diag::regress::jitter', 'джиттер — максимум вместо медианы'),
    ('diag.rs', 'bytes as f64 * 8.0 / secs / 1e6', 'bytes as f64 / secs / 1e6', 'diag::regress::mbps_units', 'байты вместо бит в Мбит/с'),
    ('diag.rs', 'const EARLY_TOLERANCE: f64 = 0.05;', 'const EARLY_TOLERANCE: f64 = 0.5;', 'diag::regress::early_stop', 'допуск «устоялась» ±50% вместо ±5%'),
    ('limits.rs', 'if !seen.insert(h) {', 'if false && !seen.insert(h) {', 'limits::regress::claude_same_answer', 'повторный ответ считается заново'),
    ('limits.rs', 'if !buf.ends_with(b"\\n") && got <= LINE_CAP {', 'if false && got <= LINE_CAP {', 'limits::regress::claude_rescan', 'недописанная строка «съедается»'),
    ('limits.rs', 'input: inp.saturating_sub(cached),', 'input: inp,', 'limits::regress::codex_schema', 'кэш Codex не вычитается из входа'),
    ('limits.rs', 'let (today, week) = (local_day(now), local_day(now - 6 * 86_400));', 'let (today, week) = (local_day(now), local_day(now - 7 * 86_400));', 'limits::regress::usage_row_window', 'окно «7 дней» — 8 дней'),
    ('footprint.rs', "f.write_all(&vec![b'*'; e - s])", "f.write_all(&vec![b'*'; e - s - 1])", 'footprint::regress::mask_file', 'маска короче ключа на байт (размер файла «плывёт»)'),
    ('footprint.rs', 'const LEFT: usize = 8;', 'const LEFT: usize = 0;', 'footprint::regress::mask_file_word_start', 'нет запаса слева на стыке проходов'),
    ('scene_anim.rs', 'const TAIL_SPEED: [u64; 3] = [1800, 600, 1000];', 'const TAIL_SPEED: [u64; 3] = [1800, 700, 1000];', 'scene_anim::regress', 'скорость хвоста у «пишет» другая'),
    ('scene_anim.rs', 'for _ in 0..=(pair[1] - b\'0\') {', 'for _ in 0..(pair[1] - b\'0\') {', 'scene_anim::regress', 'разжиматель: длина серии на единицу короче'),
    ('tray.rs', 'resets_at - now <= i64::from(minutes) * 60 / 5', 'resets_at - now <= i64::from(minutes) * 60 / 4', 'tray::regress::limit_line', 'порог «сбросится» — четверть окна'),
    ('ui.rs', 'v.sort_by_key(|p| p.1 != "bad");', 'v.sort_by_key(|p| p.1 == "bad");', 'ui::regress::problems', 'жёлтые раньше красных'),
    ('fixes.rs', 'active && fails_session', 'active || fails_session', 'fixes::regress', 'отказ при любом одном признаке'),
]


def sh(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def prepare():
    shutil.rmtree(M, ignore_errors=True)
    os.makedirs(os.path.join(M, 'art'))
    for n in ('assets', 'packaging', 'release'):  # release/ — лицензии для build.rs
        os.symlink(os.path.join(ROOT, n), os.path.join(M, n))
    os.symlink(os.path.join(ROOT, 'art', 'layers'), os.path.join(M, 'art', 'layers'))
    os.makedirs(os.path.join(M, 'engine'))
    for n in ('Cargo.toml', 'Cargo.lock', 'build.rs', 'rustfmt.toml', 'i18n', 'assets', 'vendor', 'tests', '.cargo'):
        s, d = os.path.join(ROOT, 'engine', n), os.path.join(M, 'engine', n)
        (shutil.copytree if os.path.isdir(s) else shutil.copy)(s, d)
    shutil.copytree(os.path.join(ROOT, 'engine', 'src'), os.path.join(M, 'engine', 'src'))


def main():
    only = None
    args = [a for a in sys.argv[1:] if a != '--keep']
    if args:
        only = {int(x) for x in args[0].split(',')}
    prepare()
    env = dict(os.environ, CARGO_TARGET_DIR=TARGET, CARGO_PROFILE_RELEASE_LTO='false', CARGO_PROFILE_RELEASE_CODEGEN_UNITS='16',
               CARGO_PROFILE_RELEASE_OPT_LEVEL='1', PATH=os.path.expanduser('~/.cargo/bin') + ':' + os.environ['PATH'])
    eng = os.path.join(M, 'engine')
    base = sh(['cargo', 'test', '--release', 'regress::', '--', '--skip', 'soak', '--skip', 'real_http'], cwd=eng, env=env)
    ok0 = base.returncode == 0
    print(f'без мутаций: {"зелёно" if ok0 else "КРАСНО (прогон прерван)"}')
    if not ok0:
        print(base.stdout[-2000:], base.stderr[-2000:])
        sys.exit(2)
    survived = []
    for i, (f, old, new, who, what) in enumerate(MUTS, 1):
        if only and i not in only:
            continue
        p = os.path.join(eng, 'src', f)
        src = open(p, encoding='utf-8').read()
        if src.count(old) != 1:
            print(f'{i:2}. ПРОПУСК ({f}: фрагмент найден {src.count(old)} раз): {what}')
            continue
        open(p, 'w', encoding='utf-8').write(src.replace(old, new))
        r = sh(['cargo', 'test', '--release', 'regress::', '--', '--skip', 'soak', '--skip', 'real_http'], cwd=eng, env=env)
        open(p, 'w', encoding='utf-8').write(src)
        failed = [l.split()[1] for l in r.stdout.splitlines() if l.startswith('test ') and l.endswith('FAILED')]
        hit = [t for t in failed if who in t]
        compiled = 'could not compile' not in r.stderr
        verdict = 'ПОЙМАНА' if hit else ('не поймана тестами' if compiled else 'не собралась')
        if not hit:
            survived.append((i, what))
        print(f'{i:2}. {verdict}: {what}  [{f}]  упали: {", ".join(t.split("::")[-1] for t in failed[:4])}{" …" if len(failed) > 4 else ""}')
    print(f'\nвсего мутаций {len(MUTS) if not only else len(only)}, не поймано {len(survived)}')
    if '--keep' not in sys.argv:
        shutil.rmtree(M, ignore_errors=True)
    sys.exit(1 if survived else 0)


main()
