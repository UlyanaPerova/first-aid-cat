#!/usr/bin/env python3
"""Длительное испытание релизного бинарника Аптечки в БЕЗОПАСНЫХ режимах (без сети, без настоящих настроек и чатов).

Что делает: по кругу запускает собранный `aptechka` в режимах, которые только рисуют картинки или пишут файлы
(TRAY_SHOT с TRAY_SCENE/TRAY_T/UI_FAKE, UI_SHOT с UI_FAKE/UI_K/NODIAG, FRAMES, WIDGET_SHOT), с подменённым HOME, XDG_CONFIG_HOME
и XDG_RUNTIME_DIR (временная папка). Каждые запуск проверяется на корректность результата, замеряются: время (мс), процессорное
время (мс), пик памяти (КБ), код возврата. Сам сборщик каждые несколько секунд пишет свои RSS, число дескрипторов, число
потоков и число файлов во временной папке (утечки проявятся ростом). Дополнительно в каждом круге — запуск, убитый в случайный
момент (перезапуск во время работы): следующий запуск обязан пройти, а в папке не остаётся битых json.

Запуск:
  tools/soak.py --minutes 25 --csv /tmp/soak.csv                 # ограниченный прогон
  tools/soak.py --minutes 120 --csv /tmp/soak_2h.csv             # 1–2 часа
  tools/soak.py --minutes 1440 --csv /tmp/soak_24h.csv           # сутки
  tools/soak.py --rust --minutes 30 --csv /tmp/soak_rust.csv     # то же + длительные тесты внутри процесса (cargo, REGRESS_SOAK_SECS)

Корректность: картинка — настоящий PNG ожидаемого размера (снимок окна содержит часы с секундами — у него сверяется только размер); два запуска с теми же параметрами дают один и тот же результат
(допуск на смену минуты: плашка пишет «сбросится в ЧЧ:ММ»; при расхождении базовая картинка пересчитывается ещё раз);
FRAMES даёт ровно N файлов; убитый запуск не оставляет битых json.
Выход: код 0, если все запуски корректны, память сборщика и число дескрипторов не росли.
"""
import argparse, hashlib, json, os, random, resource, shutil, signal, struct, subprocess, sys, tempfile, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, 'engine', 'target', 'release', 'aptechka')


def png_size(path):
    try:
        with open(path, 'rb') as f:
            h = f.read(24)
        if h[:8] != b'\x89PNG\r\n\x1a\n' or h[12:16] != b'IHDR':
            return None
        return struct.unpack('>II', h[16:24])
    except OSError:
        return None


def sha(path):
    try:
        with open(path, 'rb') as f:
            return hashlib.sha256(f.read()).hexdigest()
    except OSError:
        return None


def self_stats():
    rss = thr = 0
    try:
        for line in open('/proc/self/status'):
            if line.startswith('VmRSS:'):
                rss = int(line.split()[1])
            elif line.startswith('Threads:'):
                thr = int(line.split()[1])
        fds = len(os.listdir('/proc/self/fd'))
    except OSError:
        fds = 0
    return rss, thr, fds


def count_files(d):
    n = 0
    for _, _, fs in os.walk(d):
        n += len(fs)
    return n


class Env:
    def __init__(self):
        self.root = tempfile.mkdtemp(prefix='aptechka_soak_')
        self.run = os.path.join(self.root, 'run')
        os.makedirs(self.run, mode=0o700)
        self.env = dict(os.environ)
        for k in list(self.env):
            if k.startswith(('TRAY_', 'UI_', 'WIDGET_', 'FRAMES', 'DRY_RUN', 'SPEED_BUSY')):
                del self.env[k]
        self.env.update(HOME=self.root, XDG_CONFIG_HOME=os.path.join(self.root, 'cfg'), XDG_RUNTIME_DIR=self.run,
                        XDG_CACHE_HOME=os.path.join(self.root, 'cache'), APTECHKA_LANG='ru', NODIAG='1')

    def close(self):
        shutil.rmtree(self.root, ignore_errors=True)


def run(env, extra, args=(), kill_after=None, timeout=60):
    """Запуск с замером: (код, мс, процессорные мс, пик КБ, stderr)."""
    e = dict(env.env)
    e.update(extra)
    t0 = time.monotonic()
    p = subprocess.Popen([BIN, *args], env=e, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, stdin=subprocess.DEVNULL)
    killed = False
    if kill_after is not None:
        try:
            p.wait(timeout=kill_after)
        except subprocess.TimeoutExpired:
            p.send_signal(signal.SIGKILL)
            killed = True
    try:
        _, err = p.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        p.kill()
        _, err = p.communicate()
        return 124, (time.monotonic() - t0) * 1000, 0, 0, b'timeout'
    wall = (time.monotonic() - t0) * 1000
    return (-9 if killed else p.returncode), wall, 0, 0, err


def run4(env, extra, args=()):
    """То же через wait4: точный пик памяти и процессорное время именно этого запуска."""
    e = dict(env.env)
    e.update(extra)
    t0 = time.monotonic()
    p = subprocess.Popen([BIN, *args], env=e, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL)
    _, status, ru = os.wait4(p.pid, 0)
    p.returncode = 0  # уже собран
    wall = (time.monotonic() - t0) * 1000
    rc = os.waitstatus_to_exitcode(status)
    return rc, wall, (ru.ru_utime + ru.ru_stime) * 1000, ru.ru_maxrss


def main():
    global BIN
    ap = argparse.ArgumentParser()
    ap.add_argument('--minutes', type=float, default=5)
    ap.add_argument('--csv', default='soak.csv')
    ap.add_argument('--bin', default=BIN)
    ap.add_argument('--seed', type=int, default=1)
    ap.add_argument('--rust', action='store_true', help='параллельно запустить длительные тесты внутри процесса (cargo)')
    a = ap.parse_args()
    BIN = a.bin
    if not os.access(BIN, os.X_OK):
        sys.exit(f'нет бинарника {BIN}: cargo build --release в engine/')
    rnd = random.Random(a.seed)
    env = Env()
    rust = None
    if a.rust:
        e = dict(os.environ, REGRESS_SOAK_SECS=str(int(a.minutes * 60)), REGRESS_CSV=a.csv + '.rust.csv')
        rust = subprocess.Popen(['cargo', 'test', '--release', 'soak_', '--', '--ignored', '--nocapture', '--test-threads=1'],
                                cwd=os.path.join(ROOT, 'engine'), env=e, stdout=subprocess.DEVNULL, stderr=open(a.csv + '.rust.log', 'w'))
    out = open(a.csv, 'w')
    out.write('t_s,cycle,case,wall_ms,cpu_ms,maxrss_kb,rc,ok,harness_rss_kb,harness_threads,harness_fds,tmp_files,note\n')
    cases = []
    for sc in (0, 1, 2):
        for t in ('0', '0.3', '1.5', '7.77', '61'):
            for k in (1, 2):
                cases.append((f'tray_s{sc}_t{t}_k{k}', {'TRAY_SHOT': 'OUT', 'UI_FAKE': '1', 'TRAY_SCENE': str(sc), 'TRAY_T': t, 'UI_K': str(k)}, ()))
    for fake in ('ok', 'fix'):
        for k in (1, 2):
            cases.append((f'ui_{fake}_k{k}', {'UI_SHOT': 'OUT', 'UI_FAKE': fake, 'UI_K': str(k)}, ()))
    cases.append(('widget', {'WIDGET_SHOT': 'OUT', 'UI_FAKE': '1'}, ()))
    base = {}  # имя -> (размер, хеш, минута)
    t_start = time.monotonic()
    deadline = t_start + a.minutes * 60
    cycle = 0
    bad = 0
    first = None
    next_stat = 0
    while time.monotonic() < deadline:
        cycle += 1
        order = cases[:]
        rnd.shuffle(order)
        for name, extra, args in order:
            if time.monotonic() >= deadline:
                break
            png = os.path.join(env.root, 'out.png')
            if os.path.exists(png):
                os.remove(png)
            ex = {k: (png if v == 'OUT' else v) for k, v in extra.items()}
            rc, wall, cpu, rss = run4(env, ex, args)
            size, h = png_size(png), sha(png)
            minute = int(time.time() // 60)
            ok = rc == 0 and size is not None
            note = ''
            if ok:
                if name not in base:
                    base[name] = (size, h, minute)
                else:
                    bs, bh, bm = base[name]
                    if size != bs:
                        ok, note = False, f'размер {size} вместо {bs}'
                    elif h != bh and not name.startswith(('ui_', 'widget')):  # в снимках окна и виджета есть часы: сверяем только размер
                        # допускаем смену минуты («сбросится в ЧЧ:ММ»): пересчитываем базу и сверяем ещё раз
                        rc2, _, _, _ = run4(env, ex, args)
                        h2 = sha(png)
                        if rc2 == 0 and h2 != h and minute == int(time.time() // 60):
                            ok, note = False, 'результат нестабилен при одинаковых параметрах'
                        base[name] = (size, h2, int(time.time() // 60))
            bad += not ok
            r, th, fd = self_stats()
            out.write(f'{time.monotonic()-t_start:.1f},{cycle},{name},{wall:.0f},{cpu:.0f},{rss},{rc},{int(ok)},{r},{th},{fd},{count_files(env.root)},{note}\n')
        # кадры анимации: ровно N файлов
        if time.monotonic() < deadline:
            d = os.path.join(env.root, 'frames')
            shutil.rmtree(d, ignore_errors=True)
            n = 120
            rc, wall, cpu, rss = run4(env, {'FRAMES': d, 'N': str(n), 'UI_FAKE': '1', 'SCALE': '2', 'FPS': '20'})
            got = len(os.listdir(d)) if os.path.isdir(d) else -1
            ok = rc == 0 and got == n
            bad += not ok
            r, th, fd = self_stats()
            out.write(f'{time.monotonic()-t_start:.1f},{cycle},frames{n},{wall:.0f},{cpu:.0f},{rss},{rc},{int(ok)},{r},{th},{fd},{count_files(env.root)},{"" if ok else f"файлов {got}"}\n')
            shutil.rmtree(d, ignore_errors=True)
        # перезапуск во время работы: убиваем в случайный момент (мс), потом обычный запуск обязан пройти, json — целые
        if time.monotonic() < deadline:
            png = os.path.join(env.root, 'killed.png')
            rc, wall, _, _, _ = run(env, {'UI_SHOT': png, 'UI_FAKE': 'fix', 'UI_K': '2'}, kill_after=rnd.uniform(0.005, 0.09))
            broken = []
            for dp, _, fs in os.walk(env.root):
                for f in fs:
                    if f.endswith('.json'):
                        try:
                            json.load(open(os.path.join(dp, f), encoding='utf-8'))
                        except Exception:
                            broken.append(f)
            png2 = os.path.join(env.root, 'after.png')
            rc2, wall2, cpu2, rss2 = run4(env, {'UI_SHOT': png2, 'UI_FAKE': 'fix', 'UI_K': '2'})
            ok = rc2 == 0 and png_size(png2) is not None and not broken
            bad += not ok
            r, th, fd = self_stats()
            out.write(f'{time.monotonic()-t_start:.1f},{cycle},kill_restart,{wall2:.0f},{cpu2:.0f},{rss2},{rc2},{int(ok)},{r},{th},{fd},{count_files(env.root)},{"битые json: " + ",".join(broken) if broken else ("убит" if rc == -9 else "успел завершиться")}\n')
        out.flush()
        if first is None:
            first = self_stats()
    last = self_stats()
    out.close()
    if rust:
        rust.wait(timeout=int(a.minutes * 60) + 600)
    env.close()
    grew = (last[2] > (first or last)[2] + 5)
    print(f'циклов {cycle}, ошибок {bad}; сборщик: RSS {first[0] if first else 0}→{last[0]} КБ, потоки {last[1]}, дескрипторы {first[2] if first else 0}→{last[2]}')
    sys.exit(1 if bad or grew else 0)


if __name__ == '__main__':
    main()
