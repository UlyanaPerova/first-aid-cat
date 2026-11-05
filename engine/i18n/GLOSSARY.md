# English glossary and style for First Aid Cat (Аптечка)

How translation works: see the header of `engine/src/i18n.rs`. In short:
- The program keeps working in Russian internally. Text is translated when it is shown: drawn, printed, or read by a screen reader.
- Tables live in `engine/i18n/en/<module>.txt`. Strings that are never shown go in `engine/i18n/ignore/<module>.txt`.
- Check coverage with `python3 tools/i18n.py check` and `python3 tools/i18n.py missing <module>`.

## Table format
```
ru: Проверить всё
en: Check all

ru: Скорость {} Мбит/с
en: Speed {} Mbit/s

ru: Нашла {} в {}
en: Found {1} in {0}

ru: ключ|ключа|ключей
en: key|keys
```
- `{}` is a substituted value. The English side must have the same number of placeholders. Use `{0}`, `{1}` to reorder them.
- Plural forms are used by `diag::plural(n, one, few, many)`. Write `ru: one|few|many`, `en: singular|plural`.
- Escapes: `\n` is a newline and `\\` is a backslash. `{{` and `}}` are literal braces.
- Leading and trailing spaces are significant. A fragment like `"Канал "` is used in concatenation, so ALSO add the full template the user actually sees (`ru: Канал {}` / `en: Tunnel {}`). Runtime matching works on the whole displayed string: exact match first, then templates, then split by `\n`, ` · `, ` — `, `; `, `. `, `, `, `: `.
- An empty `en:` means «not translated yet».

## What NOT to translate (put it in ignore/<module>.txt)
- Strings only used in program logic: comparisons, picture layer names (`тело`, `ухо_левое`), JSON keys, matching against output of other programs.
- Russian words the code searches for in the output of system commands.
- Debug log lines starting with `[диагностика]`, `[лечение]`, `[окно]`, `[виджет]`. They are developer logs; ignore them.
- Strings inside `#[cfg(test)]` are skipped automatically.
If unsure whether a string is shown to the user, read the code around it. When in doubt, translate it, since translating a logic-only string is harmless.

## Names
| Russian | English |
|---|---|
| Аптечка (the app; any case form: Аптечку, Аптечки, Аптечке) | First Aid Cat |
| Мяудоктор | Dr. Meow |
| Справка от Мяудоктора | Medical Note from Dr. Meow |
| справка (the doctor's certificate document) | medical note |
| Мяудоктор Даня Н.Я. | Dr. Meow Danya N.Ya. |

## Menus and tabs
| Russian | English |
|---|---|
| Файл | File |
| Проверки | Checks |
| Вид | View |
| Виджет | Widget |
| Лечение (menu) | Treatment (so Alt+T does not clash with File) |
| Справка (menu) | Help |
| О программе | About |
| Журнал | Log |
| Показатели | Metrics |
| Сеть | Network |
| ИИ | AI |
| Графика | Graphics |
| Устройства | Devices |
| Темы | Themes |
| Система | System |
| Язык: как в системе | Language: system default |

## Status vocabulary
| Russian | English |
|---|---|
| Итог (summary row label) | Total (exactly this; the macOS widget compares it) |
| всё в порядке / в порядке / в норме | all good / OK / normal |
| внимание | attention |
| ошибка / беда | problem |
| ещё проверяю | still checking |
| не отвечает | not responding |
| выключен / выкл | off |
| включён / вкл | on |
| лечение, починка, починить, вылечить | fix (noun and verb) |
| проверка | check |
| Цифровой след | Digital footprint |
| ключ (API key) | key / API key |
| прокси | proxy |
| канал (VPN tunnel) | tunnel |
| сессия (AI agent session) | session |
| диск | disk |
| процессор | CPU |
| видеокарта | GPU |
| память | memory |
| загрузка (CPU load) | load |
| скорость | speed |
| пробно (DRY_RUN) | dry run |

## Units
мс → ms, с → s, мин → min, ч → h, д/дн → d, Мбит/с → Mbit/s, КБ/МБ/ГБ/ТБ → KB/MB/GB/TB, % stays %.

## Style
- The app voice is a friendly doctor cat speaking in the first person. In Russian the cat uses feminine forms («нашла», «проверила»); in English that becomes «I found», «I checked».
- Keep the tone plain, short and calm, with no jargon beyond what the Russian has. UI space is tight, so English should be no longer than the Russian when possible. Prefer short words.
- Use sentence case, not Title Case: «Check all», not «Check All».
- Quotes «…» become “…”. The em dash — stays an em dash with spaces, as in the Russian.
- Leave verbatim: commands, file paths, program names, URLs, environment variables, key names, product names (VPN client names, Claude, Codex, OpenCode, WireGuard, …).
- Jokes and quips (quips.rs, spravka.rs): translate the joke, not the words. Keep the cat character.
