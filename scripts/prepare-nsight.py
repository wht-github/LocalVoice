"""Extract the signed Nsight Systems MSI into the project (Python 3.12 + 7-Zip)."""
import hashlib
import msilib
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = ROOT / '.runtime/llama-downloads/NsightSystems-2026.5.1.161-3889610.msi'
SHA256 = '379c0a15a9cf7b8028081073fdd1d9798b9aaa618fb46c6a01785b69ed01d5f2'
DEST = ROOT / '.runtime/nsight-systems'
ARCHIVES = ROOT / '.runtime/nsight-package'
SEVEN_ZIP = Path('C:/Program Files/7-Zip/7z.exe')


def main():
    with PACKAGE.open('rb') as handle:
        if hashlib.file_digest(handle, 'sha256').hexdigest() != SHA256:
            raise RuntimeError('Nsight MSI checksum mismatch')
    subprocess.run([str(SEVEN_ZIP), 'x', str(PACKAGE), '*.cab', f'-o{ARCHIVES}', '-y', '-bso0', '-bsp0'], check=True)
    payload = ARCHIVES / 'payload'
    for cabinet in sorted(ARCHIVES.glob('*.cab')):
        subprocess.run([str(SEVEN_ZIP), 'x', str(cabinet), f'-o{payload}', '-y', '-bso0', '-bsp0'], check=True)
    database = msilib.OpenDatabase(str(PACKAGE), msilib.MSIDBOPEN_READONLY)

    def rows(table):
        view = database.OpenView('SELECT * FROM ' + table)
        view.Execute(None)
        while row := view.Fetch():
            yield [row.GetString(i) for i in range(1, row.GetFieldCount() + 1)]

    directories = {row[0]: (row[1], row[2]) for row in rows('Directory')}
    components = {row[0]: row[2] for row in rows('Component')}

    def directory(key):
        if key == 'INSTALLDIR':
            return DEST
        if key not in directories:
            return None
        parent, name = directories[key]
        base = directory(parent)
        return base / name.split('|')[-1] if base is not None else None

    copied = 0
    for row in rows('File'):
        folder = directory(components[row[1]])
        if folder is None:
            continue  # Only application files, no installer actions or system components.
        target = (folder / row[2].split('|')[-1]).resolve()
        if not target.is_relative_to(DEST.resolve()):
            raise RuntimeError(f'Unexpected MSI destination: {target}')
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(payload / row[0], target)
        copied += 1
    print(f'Extracted {copied} application files to {DEST}')


if __name__ == '__main__':
    main()
