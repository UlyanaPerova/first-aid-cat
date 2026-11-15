#!/usr/bin/env python3
"""Сквозные проверки настоящего релизного бинарника на выдуманной домашней папке (REGRESSION_TESTS.md).

Все файлы — во временной папке (HOME, XDG_CONFIG_HOME, XDG_RUNTIME_DIR подменены). Ключи в чатах/истории — выдуманные.
Проверяется то, чего не видят юнит-тесты внутри процесса:
  1. `--statusline`: в файлы лимитов попадают только `rate_limits` и `at` (остальной ввод Claude Code — нет), права 0600,
     мусорный/пустой/огромный ввод не ломает и ничего не пишет, прежняя строка состояния человека продолжает работать;
  2. `--claude-limits on/off`: настройки Claude Code правятся и возвращаются, сборка из target/ их не трогает, ссылка не трогается;
  3. с `--diag` (одно обращение к сети, как обычный запуск программы): выдуманные ключи из чатов, истории оболочки и .env
     не попадают ни в stdout, ни в stderr, ни в файлы в папках настроек и сеанса.
Запуск: tools/regress_e2e.py [--no-net] [--bin путь]
"""
import json, os, shutil, stat, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
for _s in (sys.stdout, sys.stderr):  # консоль Windows (cp1252) не печатает кириллицу — падать из-за этого нельзя
    _s.reconfigure(encoding='utf-8', errors='replace')
from regress_isolation import isolated_env, preflight  # noqa: E402

BIN = os.path.join(ROOT, 'engine', 'target', 'release', 'aptechka' + ('.exe' if os.name == 'nt' else ''))
if '--bin' in sys.argv:
    BIN = sys.argv[sys.argv.index('--bin') + 1]
NET = '--no-net' not in sys.argv

# выдуманные «ключи»: случайные, ничьи
K1 = 'sk-proj-' + 'Qw7Er9Ty2Ui4Op6As8Df1Gh3Jk5Lz0Xc9Vb7Nm5Qa3Ws1Ed'
K2 = 'ghp_' + 'Ab3Cd5Ef7Gh9Ij1Kl3Mn5Op7Qr9St1Uv3Wx5'
results = []


def check(name, ok, info=''):
    results.append((name, ok, info))
    print(('ok    ' if ok else 'ОШИБКА'), name, info)


def mk():
    root = tempfile.mkdtemp(prefix='aptechka_e2e_')
    env = isolated_env(root)
    return root, env


def run(args, env, inp=b'', timeout=60):
    p = subprocess.run([BIN, *args], env=env, input=inp, capture_output=True, timeout=timeout)
    return p.returncode, p.stdout, p.stderr


def tree(root):
    for dp, _, fs in os.walk(root):
        for f in fs:
            yield os.path.join(dp, f)


def main():
    if not os.access(BIN, os.X_OK):
        sys.exit(f'нет бинарника {BIN}')
    # ---------------- 0. изоляция: программа пишет только во временную папку ----------------
    root, env = mk()
    bad = preflight(BIN, env, root)
    check('изоляция: пробная запись осталась во временной папке, настоящие папки человека не тронуты', bad is None, bad or '')
    shutil.rmtree(root, ignore_errors=True)
    if bad or '--isolation-only' in sys.argv:
        sys.exit(1 if bad else 0)
    # ---------------- 1. строка состояния ----------------
    root, env = mk()
    sess = (os.path.join(root, 'run'), os.path.join(root, 'tmp'))
    rl = {'five_hour': {'used_percentage': 12.5, 'resets_at': 4102444800}, 'seven_day': {'used_percentage': 40, 'resets_at': 4102444800}}
    inp = json.dumps({'rate_limits': rl, 'cwd': '/home/выдумка/проект', 'transcript_path': f'/x/{K1}.jsonl', 'api_key': K2, 'model': {'id': 'x'}}).encode()
    rc, out, err = run(['--statusline'], env, inp)
    files = [p for p in tree(root) if p.endswith('claude_limits.json')]
    lim = next((p for p in files if p.startswith(sess)), None)      # копия в папке сеанса
    saved = next((p for p in files if not p.startswith(sess)), None)  # копия в папке настроек (переживает перезагрузку)
    cfg = os.path.dirname(saved) if saved else os.path.join(root, '.config', 'aptechka')
    check('statusline: код возврата 0', rc == 0, f'rc={rc}')
    check('statusline: оба файла лимитов записаны', len(files) == 2, str(len(files)))
    for p in files:
        body = open(p, encoding='utf-8').read()
        d = json.loads(body)
        check(f'statusline: в {"сеанс" if p.startswith(sess) else "настройки"} только rate_limits и at', set(d) == {'rate_limits', 'at'}, str(sorted(d)))
        check('statusline: чужие поля ввода (ключи, пути) не попали в файл', K1 not in body and K2 not in body and 'выдумка' not in body)
        check('statusline: права файла — только владелец', stat.S_IMODE(os.stat(p).st_mode) & 0o077 == 0, oct(stat.S_IMODE(os.stat(p).st_mode)))
    m0 = os.stat(lim).st_mtime_ns if os.path.exists(lim) else None
    rc, _, _ = run(['--statusline'], env, inp)
    check('statusline: те же цифры повторно — файл не переписывается', m0 is not None and os.stat(lim).st_mtime_ns == m0)
    # мусор, пустой ввод, огромный ввод
    n_before = {p: os.stat(p).st_mtime_ns for p in tree(root)}
    for name, junk in (('мусор', b'\xff\xfe not json'), ('пусто', b''), ('огромный', b'{"rate_limits":' + b'1' * (3 << 20)), ('без rate_limits', b'{"model":1}')):
        rc, _, _ = run(['--statusline'], env, junk)
        check(f'statusline: {name} — код 0, файлы не изменены', rc == 0 and {p: os.stat(p).st_mtime_ns for p in tree(root)} == n_before, f'rc={rc}')
    # цепочка: прежняя строка состояния человека
    os.makedirs(cfg, exist_ok=True)
    json.dump({'claude_statusline_prev': {'type': 'command', 'command': 'echo ПРЕЖНЯЯ'}}, open(os.path.join(cfg, 'ui.json'), 'w'))
    rc, out, _ = run(['--statusline'], env, inp)
    check('statusline: прежняя строка состояния продолжает работать', 'ПРЕЖНЯЯ'.encode() in out, out[:60].decode(errors='replace'))
    shutil.rmtree(root, ignore_errors=True)

    # ---------------- 2. подключение к настройкам Claude Code ----------------
    root, env = mk()
    cl = os.path.join(root, '.claude')
    os.makedirs(cl)
    st = os.path.join(cl, 'settings.json')
    mine = {'type': 'command', 'command': '~/my-line.sh', 'padding': 2}
    orig = {'model': 'x', 'statusLine': mine, 'hooks': {'a': [1]}}
    json.dump(orig, open(st, 'w'))
    # сборка из target/: временный путь не пишем
    rc, _, _ = run(['--claude-limits', 'on'], env)
    check('claude-limits: сборка из target/ настройки не трогает', json.load(open(st)) == orig, f'rc={rc}')
    # копия бинарника в обычное место — подключается
    bd = os.path.join(root, 'bin')
    os.makedirs(bd)
    exe = os.path.join(bd, 'aptechka')
    shutil.copy(BIN, exe)
    p = subprocess.run([exe, '--claude-limits', 'on'], env=env, capture_output=True)
    d = json.load(open(st))
    ours = d.get('statusLine', {}).get('command', '')
    check('claude-limits on: строка состояния подключена, остальное сохранено', '--statusline' in ours and d['model'] == 'x' and d['hooks'] == orig['hooks'] and d['statusLine'].get('padding') == 2, ours)
    p = subprocess.run([exe, '--claude-limits', 'on'], env=env, capture_output=True)
    check('claude-limits on повторно — без изменений', json.load(open(st)) == d)
    p = subprocess.run([exe, '--claude-limits', 'off'], env=env, capture_output=True)
    check('claude-limits off: строка человека возвращена как была', json.load(open(st)) == orig, json.dumps(json.load(open(st)))[:120])
    # настройки — ссылка: не трогаем
    real = os.path.join(root, 'real_settings.json')
    shutil.move(st, real)
    os.symlink(real, st)
    before = open(real).read()
    subprocess.run([exe, '--claude-limits', 'on'], env=env, capture_output=True)
    check('claude-limits: файл настроек — ссылка, цель не изменена', open(real).read() == before and os.path.islink(st))
    # битый JSON
    os.remove(st)
    open(st, 'w').write('{ не json')
    subprocess.run([exe, '--claude-limits', 'on'], env=env, capture_output=True)
    check('claude-limits: битый settings.json не затёрт', open(st).read() == '{ не json')
    shutil.rmtree(root, ignore_errors=True)

    # ---------------- 3. секреты в выводе и файлах ----------------
    if NET:
        root, env = mk()
        d = os.path.join(root, '.claude', 'projects', 'p')
        os.makedirs(d)
        open(os.path.join(d, 's.jsonl'), 'w').write(json.dumps({'type': 'user', 'message': {'content': f'вот ключ {K1}'}}) + '\n')
        open(os.path.join(root, '.bash_history'), 'w').write(f'export OPENAI_API_KEY={K2}\nls\n')
        open(os.path.join(root, '.env'), 'w').write(f'TOKEN={K1}\n')
        rc, out, err = run(['--diag'], env, timeout=180)
        blob = out + err
        leaks = [k for k in (K1, K2, K1[8:20], K2[4:16]) if k.encode() in blob]
        check('--diag: выдуманные ключи не в stdout/stderr', not leaks, f'rc={rc}, утечки: {len(leaks)}')
        bad = []
        for p in tree(root):
            if p.endswith(('.bash_history', '.env', 's.jsonl')):
                continue
            try:
                b = open(p, 'rb').read()
            except OSError:
                continue
            if any(k.encode() in b for k in (K1, K2, K1[8:20], K2[4:16])):
                bad.append(os.path.relpath(p, root))
        check('--diag: выдуманные ключи не в файлах настроек и сеанса', not bad, ', '.join(bad))
        shutil.rmtree(root, ignore_errors=True)
    n_bad = sum(1 for r in results if not r[1])
    print(f'\nпроверок {len(results)}, ошибок {n_bad}')
    sys.exit(1 if n_bad else 0)


main()
