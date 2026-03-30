# -*- mode: python ; coding: utf-8 -*-


a = Analysis(
    ['rustclaw_small_screen.py'],
    pathex=[],
    binaries=[],
    datas=[('assets', 'assets'), ('small_screen_markets.toml', '.'), ('signature.py', '.'), ('RustClaw480X320.png', '.'), ('longxia.png', '.')],
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
    name='rustclaw-small-screen',
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
    name='rustclaw-small-screen',
)
