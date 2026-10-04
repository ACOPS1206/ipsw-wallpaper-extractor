"""Verify actual signed app entitlements before publishing the macOS ZIP."""
import plistlib
import subprocess
import sys

if len(sys.argv) != 2:
    raise SystemExit('Usage: python3 scripts/validate_macos.py <signed.app>')
result = subprocess.run(
    ['codesign', '-d', '--entitlements', '-', '--xml', sys.argv[1]],
    check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
)
info = plistlib.loads(result.stdout)
for key in (
    'com.apple.security.app-sandbox',
    'com.apple.security.network.client',
    'com.apple.security.files.user-selected.read-write',
):
    if info.get(key) is not True:
        raise SystemExit(f'Signed app is missing required entitlement: {key}')
if info.get('com.apple.security.files.user-selected.read-only') is True:
    raise SystemExit('Signed app retains conflicting read-only file access')
print('Signed macOS app: sandbox, Apple CDN access and user-selected file writing verified.')
