"""Изоляция запусков настоящего бинарника в проверках: всё, что он пишет, должно остаться во временной папке.

2026-11-15 ранняя версия сквозных проверок на macOS не подменяла TMPDIR: выдуманные лимиты (12,5 % и 40 %, сброс в 2100 году)
попали в настоящую папку сеанса человека, и программа показала их в плашке. Отсюда правила:
  - isolated_env(): подменяются ВСЕ переменные, по которым программа находит папки, на всех системах сразу;
  - preflight(): до любых проверок один пробный запуск с меченой записью; метка обязана появиться во временной папке и
    не появиться в настоящих папках программы. Иначе настоящие файлы возвращаются как были, а проверки останавливаются.
"""
import json
import os
import subprocess
import sys

MARK = 4102444801  # «сброс» 1 января 2100 года + 1 с: такого времени сброса у настоящих лимитов не бывает
NAME = 'claude_limits.json'


def isolated_env(root):
    """Окружение, в котором программа видит только папку root (Linux, macOS, Windows)."""
    env = dict(os.environ)
    for k in list(env):
        if k.startswith(('TRAY_', 'UI_', 'WIDGET_', 'FRAMES', 'DRY_RUN', 'SPEED_BUSY', 'CLAUDE_')):
            del env[k]
    run, tmp = os.path.join(root, 'run'), os.path.join(root, 'tmp')
    os.makedirs(run, mode=0o700, exist_ok=True)
    os.makedirs(tmp, exist_ok=True)
    roam, local = os.path.join(root, 'AppData', 'Roaming'), os.path.join(root, 'AppData', 'Local')
    os.makedirs(roam, exist_ok=True)
    os.makedirs(local, exist_ok=True)
    env.update(HOME=root, USERPROFILE=root, XDG_CONFIG_HOME=os.path.join(root, '.config'), XDG_RUNTIME_DIR=run,
               XDG_CACHE_HOME=os.path.join(root, 'cache'), TMPDIR=tmp, TEMP=tmp, TMP=tmp, APPDATA=roam, LOCALAPPDATA=local,
               APTECHKA_LANG='ru')
    return env


def real_dirs():
    """Настоящие папки программы у человека, запустившего проверки (по настоящему окружению)."""
    e, h = os.environ, os.path.expanduser('~')
    d = [os.path.join(e.get('XDG_RUNTIME_DIR', ''), 'aptechka'), os.path.join(e.get('XDG_CONFIG_HOME') or os.path.join(h, '.config'), 'aptechka'),
         os.path.join(e.get('TMPDIR', '/tmp'), 'aptechka'), os.path.join(h, 'Library', 'Application Support', 'aptechka'),
         os.path.join(e.get('TEMP', ''), 'aptechka'), os.path.join(e.get('TMP', ''), 'aptechka'), os.path.join(e.get('APPDATA', ''), 'aptechka')]
    try:
        d.append(f'/tmp/aptechka-{os.getuid()}')
    except AttributeError:
        pass
    return sorted({os.path.abspath(x) for x in d if os.path.isabs(x)})


def _read(p):
    try:
        return open(p, 'rb').read()
    except OSError:
        return None


def preflight(binary, env, root):
    """Пробный запуск с меченой записью. Возвращает None, если всё осталось во временной папке, иначе текст беды
    (настоящие файлы к этому мигу уже возвращены как были)."""
    real = [os.path.join(d, NAME) for d in real_dirs() if not os.path.abspath(d).startswith(os.path.abspath(root))]
    before = {p: _read(p) for p in real}
    inp = json.dumps({'rate_limits': {'five_hour': {'used_percentage': 1, 'resets_at': MARK}}}).encode()
    try:
        subprocess.run([binary, '--statusline'], env=env, input=inp, capture_output=True, timeout=60)
    except (OSError, subprocess.TimeoutExpired) as e:
        return f'пробный запуск не удался: {e}'
    mark = str(MARK).encode()
    leaked = [p for p in real if mark in (_read(p) or b'')]
    for p in leaked:  # вернуть как было
        try:
            if before[p] is None:
                os.remove(p)
            else:
                open(p, 'wb').write(before[p])
        except OSError:
            pass
    if leaked:
        return 'программа записала пробные данные в НАСТОЯЩИЕ папки человека (возвращено как было): ' + ', '.join(leaked)
    inside = [os.path.join(d, f) for d, _, fs in os.walk(root) for f in fs if f == NAME]
    if not any(mark in (_read(p) or b'') for p in inside):
        return 'пробная запись не нашлась во временной папке — неизвестно, куда пишет программа'
    for p in inside:
        os.remove(p)
    return None


def require(binary, env, root):
    bad = preflight(binary, env, root)
    if bad:
        sys.exit('ИЗОЛЯЦИЯ НАРУШЕНА, проверки остановлены: ' + bad)
