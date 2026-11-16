#!/usr/bin/env python3
"""Перевод вживую: что на самом деле остаётся по-русски в английском интерфейсе на ЭТОЙ машине.

tools/i18n.py check сверяет строки кода с таблицей, но не видит склеек («выключен» + « (интерфейс … лежит)»)
и того, что собрано до смены языка. Здесь программа запускается по-настоящему (проверки читают систему,
ничего не меняют; настройки — во временной папке, tools/regress_isolation.py) в трёх видах:
  1. aptechka --diag с DIAG_FULL=1 — значения, подсказки, кнопки и тексты лечений всех строк;
  2. окно на английском, все вкладки (снимок без показа) — с журналом непереведённого APTECHKA_I18N_MISSING;
  3. окно, запущенное по-русски и переключённое на английский («Вид → Язык») — то же.
Печатает строки, где остались русские буквы. Названия из самой системы (звуковое устройство, программа)
переводу не подлежат — их видно глазами; код возврата 1, если что-то осталось (с --strict).

  python3 tools/i18n_live.py [--strict] [путь к aptechka]
"""
import os
import re
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
from regress_isolation import isolated_env  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
CYR = re.compile('[А-Яа-яЁё]')
TABS = 8


def run(exe, extra, switch=False):
    env = isolated_env(tempfile.mkdtemp(prefix='i18n_live_'))
    if switch:
        env.pop('APTECHKA_LANG', None)
        env['LANG'] = env['LC_ALL'] = 'ru_RU.UTF-8'
    else:
        env['APTECHKA_LANG'] = 'en'
    env.update(SCHED='0', **extra)
    return subprocess.run([exe] + (['--diag'] if 'DIAG_FULL' in extra else []), env=env, capture_output=True, text=True, timeout=600)


def main():
    args = [a for a in sys.argv[1:] if a != '--strict']
    exe = args[0] if args else os.path.join(ROOT, 'engine', 'target', 'release', 'aptechka')
    left = {}
    for line in run(exe, {'DIAG_FULL': '1'}).stdout.splitlines():
        if CYR.search(line):
            left.setdefault(line.strip(), 'проверки (--diag)')
    out = tempfile.mkdtemp(prefix='i18n_live_out_')
    for switch in (False, True):
        miss = os.path.join(out, f'missing{int(switch)}.txt')
        for tab in range(TABS):
            do = f'lang2;tab{tab}' if switch else f'tab{tab}'
            run(exe, {'UI_SHOT': os.path.join(out, f'tab{int(switch)}{tab}.png'), 'UI_DO': do, 'UI_H': '1100', 'APTECHKA_I18N_MISSING': miss}, switch)
        if os.path.exists(miss):
            for line in open(miss, encoding='utf-8'):
                left.setdefault(line.strip(), 'окно после смены языка' if switch else 'окно')
    # обрезанные для показа повторы («за час без обры…») — одна и та же строка
    keys = sorted(left)
    keys = [k for k in keys if not (k.endswith('…') and any(o != k and o.startswith(k[:-1]) for o in keys))]
    for k in keys:
        print(f'{left[k]}: {k[:300]}')
    print(f'\nосталось по-русски: {len(keys)}  (снимки вкладок: {out})')
    return 1 if keys and '--strict' in sys.argv else 0


if __name__ == '__main__':
    sys.exit(main())
