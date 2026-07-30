# -*- mode: python ; coding: utf-8 -*-

import os
import tomllib
from pathlib import Path

identity_path = Path(os.environ.get('APP_PRODUCT_IDENTITY_CONFIG', '../configs/product_identity.toml'))
identity = tomllib.loads(identity_path.read_text(encoding='utf-8'))
splash = identity['small_screen_splash_image']
datas = [('assets', 'assets'), ('small_screen_markets.toml', '.'), ('signature.py', '.'), ('longxia.png', '.')]
if Path(splash).is_file():
    datas.append((splash, '.'))

a = Analysis(
    ['agent_small_screen.py'],
    pathex=[],
    binaries=[],
    datas=datas,
    hiddenimports=['PIL.Image', 'PIL.ImageTk'],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    noarchive=False,
    optimize=0,
)
pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name='agent-small-screen',
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    console=True,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
)
coll = COLLECT(
    exe,
    a.binaries,
    a.datas,
    strip=False,
    upx=True,
    upx_exclude=[],
    name='agent-small-screen',
)
