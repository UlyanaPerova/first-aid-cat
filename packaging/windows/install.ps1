# Установка Аптечки для текущего пользователя Windows — без прав администратора.
#   install.ps1              — поставить: программа, ярлыки в меню «Пуск», автозапуск виджета, строка в «Приложениях»
#   install.ps1 -NoWidget    — без автозапуска виджета
#   install.ps1 -Desktop     — ещё и ярлык на рабочий стол (если он уже есть — обновится и без флага)
#   install.ps1 -Uninstall   — убрать всё, что поставили (настройки остаются)
# Проще всего — двойной щелчок по install.cmd рядом. Программу берёт рядом со скриптом (aptechka.exe из архива релиза).
param([switch]$NoWidget, [switch]$Desktop, [switch]$Uninstall)
$ErrorActionPreference = 'Stop'
$Dir   = Join-Path $env:LOCALAPPDATA 'Programs\Aptechka'
$Exe   = Join-Path $Dir 'aptechka.exe'
# названия ярлыков — на языке Windows (как и сама программа выбирает язык)
$Ru    = @('ru', 'be', 'kk', 'ky') -contains (Get-UICulture).TwoLetterISOLanguageName
$Name  = if ($Ru) { 'Аптечка' } else { 'First Aid Cat' }
$Menu  = Join-Path ([Environment]::GetFolderPath('Programs')) $Name
$Start = [Environment]::GetFolderPath('Startup')
$Desk  = [Environment]::GetFolderPath('Desktop')
$Auto  = Join-Path $Start ($Name + ' — ' + $(if ($Ru) { 'виджет' } else { 'widget' }) + '.lnk')
$DLnk  = Join-Path $Desk ($Name + '.lnk')
$Reg   = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Aptechka'
function Say($t) { Write-Host "[$(L 'установка' 'install')] $t" }
function L($textRu, $textEn) { if ($Ru) { $textRu } else { $textEn } }
$Widget = $Name + ' — ' + (L 'виджет' 'widget')
# оба варианта названий — для удаления, если язык Windows с тех пор сменили
$AllNames = @('Аптечка', 'First Aid Cat')

function Stop-Aptechka {
    Get-Process aptechka -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Exe } | ForEach-Object {
        $_.Kill(); $_.WaitForExit(5000) | Out-Null
    }
}

function New-Link($path, $args_) {
    $sh = New-Object -ComObject WScript.Shell
    # WScript.Shell не сохраняет в путь с кириллицей — сохраняем под латинским именем и переименовываем
    $tmp = Join-Path $env:TEMP ("aptechka-" + [guid]::NewGuid().ToString('N') + '.lnk')
    $l = $sh.CreateShortcut($tmp)
    $l.TargetPath = $Exe; $l.Arguments = $args_; $l.WorkingDirectory = $Dir
    $l.IconLocation = "$Exe,0"   # подсказку (Description) не ставим: WScript.Shell пишет её без кириллицы
    $l.Save()
    Move-Item -Force $tmp $path
}

if ($Uninstall) {
    Stop-Aptechka
    foreach ($n in $AllNames) {
        Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $Start "$n — виджет.lnk"), (Join-Path $Start "$n — widget.lnk"), (Join-Path $Desk "$n.lnk")
        Remove-Item -Recurse -Force -ErrorAction SilentlyContinue (Join-Path ([Environment]::GetFolderPath('Programs')) $n)
    }
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $Dir, $Reg
    Say (L 'Аптечка убрана (настройки в %APPDATA%\aptechka остались)' 'First Aid Cat removed (settings in %APPDATA%\aptechka are kept)')
    exit 0
}

$Src = Join-Path $PSScriptRoot 'aptechka.exe'
if (-not (Test-Path $Src)) { Say ((L 'не нашла aptechka.exe рядом со скриптом' 'aptechka.exe not found next to the script') + " ($PSScriptRoot)"); exit 1 }

Stop-Aptechka
New-Item -ItemType Directory -Force $Dir, $Menu | Out-Null
Copy-Item -Force $Src $Exe
Copy-Item -Force $PSCommandPath (Join-Path $Dir 'install.ps1')   # чтобы удалить из «Приложений»
Say ((L 'программа' 'program') + ": $Exe")

New-Link (Join-Path $Menu "$Name.lnk") ''
New-Link (Join-Path $Menu "$Widget.lnk") '--widget'
Say ((L 'ярлыки в меню «Пуск»' 'Start menu shortcuts') + ": $Name, $Widget")

if ($NoWidget) { Remove-Item -Force -ErrorAction SilentlyContinue $Auto }
else { New-Link $Auto '--widget'; Say (L 'виджет будет запускаться при входе' 'the widget will start at sign-in') }

if ($Desktop -or (Test-Path $DLnk)) { New-Link $DLnk ''; Say (L 'ярлык на рабочем столе' 'desktop shortcut') }

# строка в «Параметры → Приложения», чтобы удалить можно было и оттуда
New-Item -Force $Reg | Out-Null
$ver = (Get-Item $Exe).VersionInfo.ProductVersion; if (-not $ver) { $ver = '0.9.0' }
$un = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$Dir\install.ps1`" -Uninstall"
@{ DisplayName = $Name; DisplayIcon = "$Exe,0"; DisplayVersion = $ver; Publisher = 'First Aid Cat';
   InstallLocation = $Dir; UninstallString = $un; NoModify = 1; NoRepair = 1 }.GetEnumerator() | ForEach-Object {
    $t = if ($_.Value -is [int]) { 'DWord' } else { 'String' }
    New-ItemProperty -Force -Path $Reg -Name $_.Key -Value $_.Value -PropertyType $t | Out-Null
}

if (-not $NoWidget) { Start-Process $Exe '--widget' -WorkingDirectory $Dir }
Say (L 'готово. Аптечка — в меню «Пуск».' 'done. First Aid Cat is in the Start menu.')
