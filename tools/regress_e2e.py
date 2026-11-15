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
BIN = os.path.join(ROOT, 'engine', 'target', 'release', 'aptechka')
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


def real_files():
    """Настоящие файлы лимитов человека (папка сеанса и копия в настройках) — их проверки трогать не должны.

    2026-11-15 ранняя версия этого скрипта на macOS не подменяла TMPDIR, и выдуманные лимиты (12,5 % и 40 %, сброс в 2100 году)
    попали в настоящую папку сеанса: программа показала их человеку. Поэтому снимок «до» сверяется с «после»."""
    h = os.path.expanduser('~')
    dirs = [os.path.join(os.environ.get('TMPDIR', '/tmp'), 'aptechka'), os.path.join(h, 'Library/Application Support/aptechka'),
            os.path.join(os.environ.get('XDG_RUNTIME_DIR', '/nonexistent'), 'aptechka'),
            os.path.join(os.environ.get('XDG_CONFIG_HOME', os.path.join(h, '.config')), 'aptechka')]
    out = {}
    for d in dirs:
        p = os.path.join(d, 'claude_limits.json')
        try:
            out[p] = open(p, 'rb').read()
        except OSError:
            out[p] = None
    return out


def mk():
    root = tempfile.mkdtemp(prefix='aptechka_e2e_')
    run = os.path.join(root, 'run')
    os.makedirs(run, mode=0o700)
    env = dict(os.environ)
    for k in list(env):
        if k.startswith(('TRAY_', 'UI_', 'WIDGET_', 'FRAMES', 'DRY_RUN', 'SPEED_BUSY', 'CLAUDE_')):
            del env[k]
    tmp = os.path.join(root, 'tmp')
    os.makedirs(tmp)
    # TMPDIR — для macOS (папка сеанса там берётся из неё), XDG_RUNTIME_DIR — для Linux
    env.update(HOME=root, XDG_CONFIG_HOME=os.path.join(root, '.config'), XDG_RUNTIME_DIR=run, TMPDIR=tmp, APTECHKA_LANG='ru')
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
    touched = [p for p, v in real_files().items() if v != BEFORE[p] and (v is None or b'4102444800' in v)]
    check('настоящие файлы лимитов человека не тронуты', not touched, ', '.join(os.path.basename(os.path.dirname(os.path.dirname(p))) for p in touched))
    n_bad = sum(1 for r in results if not r[1])
    print(f'\nпроверок {len(results)}, ошибок {n_bad}')
    sys.exit(1 if n_bad else 0)


BEFORE = real_files()
main()
